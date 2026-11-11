// RepoStacks window — a thin Tauri shell over repostacks_core. Every
// capability is a core operation first; these commands only adapt them.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dpapi;

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::cache_policy::CachePolicy;
use repostacks_core::catalog::Catalog;
use repostacks_core::download::{
    enqueue_download, plan_download, run_pending, suggest_asset_patterns,
};
use repostacks_core::http::{CurlDownloader, CurlFetcher, Fetcher};
use repostacks_core::catalog::RepoEntry;
use repostacks_core::net::{
    auth_for_token, check_entries_parallel, fetch_release_assets, refresh_entries_parallel,
    RefreshOutcome,
};
use repostacks_core::queue::{JobStatus, Queue};
use repostacks_core::user_import::{
    apply_user_import, fetch_user_repos, Archived, Forks, Ownership, UserRepoFilter, Visibility,
};
use repostacks_core::search::{execute as search_execute, Query};
use repostacks_core::view::{status_of, Column, Density, GroupBy, Layout, Settings, Theme};
use repostacks_core::RepoRef;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

struct Inner {
    catalog: Catalog,
    settings: Settings,
    queue: Queue,
    activity: Vec<String>,
    last_atom_run: u64,
    last_metadata_run: u64,
}

struct AppState {
    inner: Mutex<Inner>,
    data_dir: PathBuf,
    exe_dir: PathBuf,
}

impl AppState {
    fn catalog_path(&self) -> PathBuf {
        env_or("REPOSTACKS_DATA", self.data_dir.join("catalog.json"))
    }
    fn settings_path(&self) -> PathBuf {
        env_or("REPOSTACKS_SETTINGS", self.data_dir.join("settings.json"))
    }
    fn queue_path(&self) -> PathBuf {
        env_or("REPOSTACKS_QUEUE", self.data_dir.join("queue.json"))
    }
    fn downloads_root(&self, settings: &Settings) -> PathBuf {
        let dir = PathBuf::from(&settings.download_dir);
        if dir.is_absolute() {
            dir
        } else {
            self.exe_dir.join(dir)
        }
    }
    fn save_catalog(&self, inner: &Inner) -> Result<(), String> {
        inner
            .catalog
            .save(&self.catalog_path())
            .map_err(|e| format!("save catalogue: {e}"))
    }
    fn save_settings(&self, inner: &Inner) -> Result<(), String> {
        inner
            .settings
            .save(&self.settings_path())
            .map_err(|e| format!("save settings: {e}"))
    }
    fn save_queue(&self, inner: &Inner) -> Result<(), String> {
        inner
            .queue
            .save(&self.queue_path())
            .map_err(|e| format!("save queue: {e}"))
    }
    fn token(&self, inner: &Inner) -> Option<String> {
        if let Ok(t) = std::env::var("REPOSTACKS_TOKEN") {
            if !t.is_empty() {
                return Some(t);
            }
        }
        if inner.settings.token_protected.is_empty() {
            return None;
        }
        dpapi::unprotect_token(&inner.settings.token_protected).ok()
    }
}

fn env_or(key: &str, fallback: PathBuf) -> PathBuf {
    match std::env::var_os(key) {
        Some(p) => PathBuf::from(p),
        None => fallback,
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Copy the fields a metadata refresh owns from a refreshed clone back
/// into the stored entry — the stored entry's other fields (shelves are
/// separate; rules, standing order, installed tag) may have changed
/// while the network work ran without the state lock held.
fn apply_refreshed(stored: &mut RepoEntry, refreshed: &RepoEntry) {
    stored.description = refreshed.description.clone();
    stored.stars = refreshed.stars;
    stored.language = refreshed.language.clone();
    stored.license = refreshed.license.clone();
    stored.topics = refreshed.topics.clone();
    stored.etag = refreshed.etag.clone();
    stored.last_metadata_fetch = refreshed.last_metadata_fetch;
}

fn log(inner: &mut Inner, text: String) {
    let secs = now_unix() % 86_400;
    let line = format!("{:02}:{:02}  {}", secs / 3600, (secs % 3600) / 60, text);
    inner.activity.push(line);
    if inner.activity.len() > 200 {
        inner.activity.remove(0);
    }
}

// ---------- DTOs ----------

#[derive(Serialize)]
struct EntryDto {
    full_name: String,
    owner: String,
    name: String,
    description: String,
    stars: u64,
    language: String,
    license: String,
    topics: Vec<String>,
    latest_tag: String,
    installed_tag: String,
    status: String,
    auto_download: bool,
    asset_patterns: Vec<String>,
    shelves: Vec<String>,
}

fn entry_dto(e: &repostacks_core::catalog::RepoEntry, catalog: &Catalog) -> EntryDto {
    EntryDto {
        full_name: e.repo.full_name(),
        owner: e.repo.owner.clone(),
        name: e.repo.name.clone(),
        description: e.description.clone(),
        stars: e.stars,
        language: e.language.clone(),
        license: e.license.clone(),
        topics: e.topics.clone(),
        latest_tag: e.latest_tag.clone(),
        installed_tag: e.installed_tag.clone(),
        status: status_of(e).to_string(),
        auto_download: e.auto_download,
        asset_patterns: e.asset_patterns.clone(),
        shelves: catalog
            .shelves
            .iter()
            .filter(|(_, m)| m.contains(&e.repo.full_name()))
            .map(|(n, _)| n.clone())
            .collect(),
    }
}

#[derive(Serialize)]
struct FacetDto {
    name: String,
    count: usize,
}

#[derive(Serialize)]
struct OverviewDto {
    volumes: usize,
    installed: usize,
    updates: usize,
    queued_jobs: usize,
    has_token: bool,
    download_dir: String,
}

#[derive(Serialize)]
struct CheckDto {
    repo: String,
    latest: String,
    changed: bool,
    error: String,
}

#[derive(Serialize)]
struct JobDto {
    id: u64,
    repo: String,
    tag: String,
    asset_name: String,
    status: String,
    bytes_done: u64,
    bytes_total: u64,
    percent: f64,
    error: String,
}

// ---------- Catalogue commands ----------

#[tauri::command]
fn overview(state: State<'_, AppState>) -> Result<OverviewDto, String> {
    let inner = state.inner.lock().unwrap();
    Ok(OverviewDto {
        volumes: inner.catalog.entries.len(),
        installed: inner
            .catalog
            .entries
            .iter()
            .filter(|e| e.is_installed())
            .count(),
        updates: inner
            .catalog
            .entries
            .iter()
            .filter(|e| e.update_available())
            .count(),
        queued_jobs: inner.queue.active_count(),
        has_token: state.token(&inner).is_some(),
        download_dir: state.downloads_root(&inner.settings).display().to_string(),
    })
}

#[tauri::command]
fn find_entries(state: State<'_, AppState>, query: String) -> Result<Vec<EntryDto>, String> {
    let inner = state.inner.lock().unwrap();
    let q = Query::parse(&query);
    let hits = search_execute(&inner.catalog, &q, now_unix());
    Ok(hits.iter().map(|e| entry_dto(e, &inner.catalog)).collect())
}

#[tauri::command]
fn facets(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().unwrap();
    let map = |pairs: Vec<(String, usize)>| {
        pairs
            .into_iter()
            .map(|(name, count)| FacetDto { name, count })
            .collect::<Vec<_>>()
    };
    Ok(serde_json::json!({
        "languages": map(inner.catalog.language_facets()),
        "topics": map(inner.catalog.topic_facets()),
        "licenses": map(inner.catalog.license_facets()),
        "owners": map(inner.catalog.owner_facets()),
    }))
}

#[tauri::command]
fn add_repo(state: State<'_, AppState>, input: String) -> Result<EntryDto, String> {
    let repo = RepoRef::parse(&input).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    inner.catalog.add(repo.clone());
    state.save_catalog(&inner)?;
    let entry = inner.catalog.get(&repo).expect("just added").clone();
    Ok(entry_dto(&entry, &inner.catalog))
}

#[tauri::command]
fn remove_repo(state: State<'_, AppState>, repo: String) -> Result<(), String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    inner.catalog.remove(&repo);
    state.save_catalog(&inner)
}

#[tauri::command]
fn import_apply(
    state: State<'_, AppState>,
    text: String,
    shelf: Option<String>,
) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().unwrap();
    let report = inner.catalog.import_list(&text);
    if let Some(shelf) = shelf {
        if !shelf.trim().is_empty() {
            for name in &report.added {
                if let Ok(repo) = RepoRef::parse(name) {
                    inner.catalog.add_to_shelf(shelf.trim(), &repo);
                }
            }
        }
    }
    log(
        &mut inner,
        format!(
            "Imported {} volume(s), {} duplicate(s), {} invalid",
            report.added.len(),
            report.duplicates.len(),
            report.invalid.len()
        ),
    );
    state.save_catalog(&inner)?;
    Ok(serde_json::json!({
        "added": report.added,
        "duplicates": report.duplicates,
        "invalid": report.invalid.iter().map(|(l, w)| serde_json::json!([l, w])).collect::<Vec<_>>(),
    }))
}

/// Open a URL or folder with the OS default handler. The URL is built
/// from a validated RepoRef and the folder from our own layout — never
/// from free-form input.
fn open_external(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if target.starts_with("https://") {
            std::process::Command::new("rundll32")
                .args(["url.dll,FileProtocolHandler", target])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("open browser: {e}"))?;
        } else {
            std::process::Command::new("explorer")
                .arg(target)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| format!("open folder: {e}"))?;
        }
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(opener)
            .arg(target)
            .spawn()
            .map_err(|e| format!("open: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
fn open_repo_page(repo: String) -> Result<(), String> {
    let r = RepoRef::parse(&repo).map_err(|e| e.0)?;
    open_external(&format!("https://github.com/{}/{}", r.owner, r.name))
}

#[tauri::command]
fn open_download_folder(state: State<'_, AppState>, repo: String) -> Result<(), String> {
    let r = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let root = {
        let inner = state.inner.lock().unwrap();
        state.downloads_root(&inner.settings)
    };
    let dir = root.join(format!("{}-{}", r.owner, r.name));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    open_external(&dir.to_string_lossy())
}

/// The saved searches from settings, as name → query pairs.
#[tauri::command]
fn saved_list(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().unwrap();
    Ok(serde_json::to_value(&inner.settings.saved_searches)
        .map_err(|e| format!("encode saved searches: {e}"))?)
}

#[tauri::command]
fn saved_save(
    state: State<'_, AppState>,
    name: String,
    query: String,
) -> Result<serde_json::Value, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A saved search needs a name.".into());
    }
    let mut inner = state.inner.lock().unwrap();
    inner
        .settings
        .saved_searches
        .insert(name, query.trim().to_string());
    state.save_settings(&inner)?;
    Ok(serde_json::to_value(&inner.settings.saved_searches)
        .map_err(|e| format!("encode saved searches: {e}"))?)
}

#[tauri::command]
fn saved_delete(state: State<'_, AppState>, name: String) -> Result<serde_json::Value, String> {
    let mut inner = state.inner.lock().unwrap();
    inner.settings.saved_searches.remove(&name);
    state.save_settings(&inner)?;
    Ok(serde_json::to_value(&inner.settings.saved_searches)
        .map_err(|e| format!("encode saved searches: {e}"))?)
}

#[derive(serde::Deserialize)]
struct UserImportDto {
    username: String,
    ownership: String,
    forks: String,
    archived: String,
    visibility: String,
    dry_run: bool,
}

/// Import a GitHub user's projects. The listing fetch runs without the
/// state lock; the apply merges under it. `dry_run` previews the counts
/// and adds nothing.
#[tauri::command]
fn import_user(state: State<'_, AppState>, input: UserImportDto) -> Result<serde_json::Value, String> {
    let filter = UserRepoFilter {
        ownership: match input.ownership.as_str() {
            "all" => Ownership::All,
            "owned" => Ownership::Owned,
            "member" => Ownership::Member,
            other => return Err(format!("unknown ownership filter '{other}'")),
        },
        forks: match input.forks.as_str() {
            "all" => Forks::All,
            "forks" => Forks::ForksOnly,
            "sources" => Forks::SourcesOnly,
            other => return Err(format!("unknown fork filter '{other}'")),
        },
        archived: match input.archived.as_str() {
            "exclude" => Archived::Exclude,
            "include" => Archived::Include,
            "only" => Archived::Only,
            other => return Err(format!("unknown archived filter '{other}'")),
        },
        visibility: match input.visibility.as_str() {
            "all" => Visibility::All,
            "public" => Visibility::PublicOnly,
            "private" => Visibility::PrivateOnly,
            other => return Err(format!("unknown visibility filter '{other}'")),
        },
    };
    let token = {
        let inner = state.inner.lock().unwrap();
        state.token(&inner)
    };
    let fetcher = CurlFetcher::default();
    let (repos, spent) = fetch_user_repos(&fetcher, &input.username, token.as_deref())?;
    let mut inner = state.inner.lock().unwrap();
    let report = apply_user_import(
        &mut inner.catalog,
        &repos,
        &filter,
        &input.username,
        now_unix(),
        input.dry_run,
    );
    if !input.dry_run && !report.added.is_empty() {
        log(
            &mut inner,
            format!(
                "Imported {} project(s) from {} ({} already catalogued)",
                report.added.len(),
                input.username,
                report.duplicates.len()
            ),
        );
        state.save_catalog(&inner)?;
    }
    Ok(serde_json::json!({
        "listed": repos.len(),
        "added": report.added,
        "duplicates": report.duplicates,
        "quota": spent,
    }))
}

#[tauri::command]
fn list_shelves(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let inner = state.inner.lock().unwrap();
    let shelves: Vec<_> = inner
        .catalog
        .shelves
        .iter()
        .map(|(name, members)| serde_json::json!({ "name": name, "members": members }))
        .collect();
    Ok(serde_json::json!(shelves))
}

#[tauri::command]
fn shelf_add(state: State<'_, AppState>, shelf: String, repo: String) -> Result<(), String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    inner.catalog.add_to_shelf(&shelf, &repo);
    state.save_catalog(&inner)
}

#[tauri::command]
fn shelf_remove(state: State<'_, AppState>, shelf: String, repo: String) -> Result<(), String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    inner.catalog.remove_from_shelf(&shelf, &repo);
    state.save_catalog(&inner)
}

#[tauri::command]
fn set_rules(
    state: State<'_, AppState>,
    repo: String,
    patterns: Vec<String>,
) -> Result<(), String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    let entry = inner
        .catalog
        .get_mut(&repo)
        .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
    entry.asset_patterns = patterns
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    state.save_catalog(&inner)
}

#[tauri::command]
fn set_auto(state: State<'_, AppState>, repo: String, on: bool) -> Result<(), String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    let entry = inner
        .catalog
        .get_mut(&repo)
        .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
    entry.auto_download = on;
    state.save_catalog(&inner)
}

// ---------- Network commands ----------

#[tauri::command]
fn check_all(state: State<'_, AppState>) -> Result<Vec<CheckDto>, String> {
    // Snapshot, then run the checks in parallel WITHOUT holding the
    // state lock — the rest of the app stays responsive while the
    // network works. Results merge back under the lock at the end.
    let (entries, concurrency) = {
        let inner = state.inner.lock().unwrap();
        (
            inner.catalog.entries.clone(),
            inner.settings.concurrency as usize,
        )
    };
    let fetcher = CurlFetcher::default();
    let now = now_unix();
    let results = check_entries_parallel(&fetcher, &entries, now, concurrency);
    let mut inner = state.inner.lock().unwrap();
    let mut out = Vec::new();
    for r in &results {
        if let Some(entry) = inner.catalog.get_mut(&r.repo) {
            entry.latest_tag = r.entry.latest_tag.clone();
            entry.last_release_check = r.entry.last_release_check;
        }
        match &r.outcome {
            Ok(o) => {
                if o.changed {
                    log(&mut inner, format!("{}: new release {}", r.repo, o.latest_tag));
                }
                out.push(CheckDto {
                    repo: r.repo.full_name(),
                    latest: o.latest_tag.clone(),
                    changed: o.changed,
                    error: String::new(),
                });
            }
            Err(msg) => out.push(CheckDto {
                repo: r.repo.full_name(),
                latest: String::new(),
                changed: false,
                error: msg.clone(),
            }),
        }
    }
    inner.last_atom_run = now;
    log(
        &mut inner,
        "Checked all volumes via Atom feeds (0 API calls)".into(),
    );
    state.save_catalog(&inner)?;
    Ok(out)
}

#[tauri::command]
fn refresh_all(state: State<'_, AppState>, force: bool) -> Result<serde_json::Value, String> {
    // Same snapshot pattern as check_all: the REST calls run on clones
    // without the state lock, then only the refreshed fields merge back.
    let (entries, token, concurrency) = {
        let inner = state.inner.lock().unwrap();
        (
            inner.catalog.entries.clone(),
            state.token(&inner),
            inner.settings.concurrency as usize,
        )
    };
    let policy = CachePolicy::for_auth(auth_for_token(token.as_deref()));
    let fetcher = CurlFetcher::default();
    let now = now_unix();
    let (mut updated, mut unchanged, mut skipped, mut spent) = (0, 0, 0, 0u32);
    let mut errors = Vec::new();
    let mut refreshed: Vec<RepoEntry> = Vec::new();
    // Parallel, bounded by the concurrency setting — a sequential pass
    // over a large catalogue costs one round-trip per repo.
    let results = refresh_entries_parallel(
        &fetcher,
        &entries,
        &policy,
        token.as_deref(),
        now,
        force,
        concurrency,
    );
    for r in results {
        match r.outcome {
            Ok(RefreshOutcome::Updated { quota_spent }) => {
                updated += 1;
                spent += quota_spent;
                refreshed.push(r.entry);
            }
            Ok(RefreshOutcome::NotModified { quota_spent }) => {
                unchanged += 1;
                spent += quota_spent;
                refreshed.push(r.entry);
            }
            Ok(RefreshOutcome::SkippedNotDue) => skipped += 1,
            Err(msg) => errors.push(format!("{}: {msg}", r.repo)),
        }
    }
    let mut inner = state.inner.lock().unwrap();
    for entry in &refreshed {
        if let Some(stored) = inner.catalog.get_mut(&entry.repo) {
            apply_refreshed(stored, entry);
        }
    }
    inner.last_metadata_run = now;
    log(
        &mut inner,
        format!("Metadata refresh: {updated} updated, {unchanged} unchanged, {spent} API call(s)"),
    );
    state.save_catalog(&inner)?;
    Ok(serde_json::json!({
        "updated": updated, "unchanged": unchanged, "skipped": skipped,
        "spent": spent, "limit": policy.auth.hourly_rest_limit(), "errors": errors,
    }))
}

#[tauri::command]
fn release_history(_state: State<'_, AppState>, repo: String) -> Result<serde_json::Value, String> {
    let repo = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let fetcher = CurlFetcher::default();
    let resp = fetcher.get(&repo.releases_atom_url(), None, None)?;
    if resp.status != 200 {
        return Err(format!("feed for {repo} answered HTTP {}", resp.status));
    }
    let notes = parse_releases_atom(&resp.body);
    Ok(serde_json::json!(notes
        .iter()
        .map(|n| serde_json::json!({ "tag": n.tag, "title": n.title, "updated": n.updated }))
        .collect::<Vec<_>>()))
}

#[tauri::command]
fn release_assets(
    state: State<'_, AppState>,
    repo: String,
    tag: String,
) -> Result<serde_json::Value, String> {
    let repo_ref = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let inner = state.inner.lock().unwrap();
    let token = state.token(&inner);
    let entry = inner
        .catalog
        .get(&repo_ref)
        .ok_or_else(|| format!("{repo} is not in the catalogue"))?
        .clone();
    drop(inner);
    let fetcher = CurlFetcher::default();
    let (assets, spent) = fetch_release_assets(&fetcher, &repo_ref, &tag, token.as_deref())?;
    let (pick, reason) = plan_download(&entry, &tag, &assets, std::path::Path::new("."))
        .map(|p| (p.asset_name, p.reason))
        .unwrap_or_default();
    Ok(serde_json::json!({
        "assets": assets.iter().map(|a| serde_json::json!({
            "name": a.name, "size": a.size, "digest": a.digest,
        })).collect::<Vec<_>>(),
        "pick": pick,
        "reason": reason,
        "suggested": suggest_asset_patterns(&assets, &tag),
        "quota": spent,
    }))
}

// ---------- Download commands ----------

fn run_queue_locked(state: &AppState, inner: &mut Inner) -> Vec<String> {
    let token = state.token(inner);
    let root = state.downloads_root(&inner.settings);
    let downloader = CurlDownloader::default();
    let concurrency = inner.settings.concurrency as usize;
    let outcomes = run_pending(
        &mut inner.queue,
        &mut inner.catalog,
        &downloader,
        &root,
        token.as_deref(),
        concurrency,
    );
    let mut lines = Vec::new();
    for o in &outcomes {
        if o.status == JobStatus::Done {
            log(inner, format!("Downloaded → {}", o.final_path.display()));
            lines.push(format!("done: {}", o.final_path.display()));
        } else {
            log(inner, format!("Download failed: {}", o.error));
            lines.push(format!("failed: {}", o.error));
        }
    }
    let _ = state.save_queue(inner);
    let _ = state.save_catalog(inner);
    lines
}

#[tauri::command]
fn download_repo(
    state: State<'_, AppState>,
    repo: String,
    tag: Option<String>,
) -> Result<String, String> {
    let repo_ref = RepoRef::parse(&repo).map_err(|e| e.0)?;
    let mut inner = state.inner.lock().unwrap();
    let token = state.token(&inner);
    let entry = inner
        .catalog
        .get(&repo_ref)
        .ok_or_else(|| format!("{repo} is not in the catalogue"))?
        .clone();
    let tag = tag
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| entry.latest_tag.clone());
    if tag.is_empty() {
        return Err(format!(
            "no tag known for {repo} — check for releases first"
        ));
    }
    let fetcher = CurlFetcher::default();
    let (assets, _spent) = fetch_release_assets(&fetcher, &repo_ref, &tag, token.as_deref())?;
    let root = state.downloads_root(&inner.settings);
    let id = enqueue_download(&mut inner.queue, &entry, &tag, &assets, &root)?;
    log(&mut inner, format!("Queued #{id}: {repo} {tag}"));
    let lines = run_queue_locked(&state, &mut inner);
    state.save_queue(&inner)?;
    lines
        .into_iter()
        .find(|l| l.starts_with("done:"))
        .ok_or_else(|| "download did not complete — see the queue".to_string())
}

#[tauri::command]
fn queue_list(state: State<'_, AppState>) -> Result<Vec<JobDto>, String> {
    let inner = state.inner.lock().unwrap();
    Ok(inner
        .queue
        .jobs
        .iter()
        .map(|j| JobDto {
            id: j.id,
            repo: j.repo.full_name(),
            tag: j.tag.clone(),
            asset_name: j.asset_name.clone(),
            status: j.status.label().to_string(),
            bytes_done: j.bytes_done,
            bytes_total: j.bytes_total,
            percent: j.progress_percent(),
            error: j.error.clone(),
        })
        .collect())
}

#[tauri::command]
fn queue_run(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let mut inner = state.inner.lock().unwrap();
    Ok(run_queue_locked(&state, &mut inner))
}

#[tauri::command]
fn queue_pause(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.queue.pause(id) {
        state.save_queue(&inner)
    } else {
        Err(format!("job {id} cannot pause now"))
    }
}

#[tauri::command]
fn queue_resume(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.queue.resume(id) {
        state.save_queue(&inner)
    } else {
        Err(format!("job {id} cannot resume now"))
    }
}

#[tauri::command]
fn queue_cancel(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.queue.cancel(id) {
        state.save_queue(&inner)
    } else {
        Err(format!("job {id} cannot cancel now"))
    }
}

#[tauri::command]
fn queue_retry(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.queue.retry(id) {
        state.save_queue(&inner)
    } else {
        Err(format!("job {id} is not failed — only failed jobs retry"))
    }
}

#[tauri::command]
fn queue_clear(state: State<'_, AppState>) -> Result<usize, String> {
    let mut inner = state.inner.lock().unwrap();
    let n = inner.queue.clear_finished();
    state.save_queue(&inner)?;
    Ok(n)
}

// ---------- View & config commands ----------

#[derive(Serialize, serde::Deserialize)]
struct ViewDto {
    layout: String,
    theme: String,
    density: String,
    group_by: String,
    columns: Vec<String>,
    show_facets: bool,
}

#[tauri::command]
fn get_view(state: State<'_, AppState>) -> Result<ViewDto, String> {
    let inner = state.inner.lock().unwrap();
    let v = &inner.settings.view;
    Ok(ViewDto {
        layout: v.layout.name().to_string(),
        theme: v.theme.name().to_string(),
        density: v.density.name().to_string(),
        group_by: v.group_by.name().to_string(),
        columns: v.columns.iter().map(|c| c.name().to_string()).collect(),
        show_facets: v.show_facets,
    })
}

#[tauri::command]
fn set_view(state: State<'_, AppState>, view: ViewDto) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    let v = &mut inner.settings.view;
    if let Some(l) = Layout::from_name(&view.layout) {
        v.layout = l;
    }
    if let Some(t) = Theme::from_name(&view.theme) {
        v.theme = t;
    }
    if let Some(d) = Density::from_name(&view.density) {
        v.density = d;
    }
    if let Some(g) = GroupBy::from_name(&view.group_by) {
        v.group_by = g;
    }
    let cols: Vec<Column> = view
        .columns
        .iter()
        .filter_map(|c| Column::from_name(c))
        .collect();
    if !cols.is_empty() {
        v.columns = cols;
    }
    v.show_facets = view.show_facets;
    state.save_settings(&inner)
}

#[derive(Serialize, serde::Deserialize)]
struct ConfigDto {
    download_dir: String,
    concurrency: u32,
    atom_poll_secs: u64,
    metadata_poll_secs: u64,
    check_on_start: bool,
    has_token: bool,
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Result<ConfigDto, String> {
    let inner = state.inner.lock().unwrap();
    Ok(ConfigDto {
        download_dir: inner.settings.download_dir.clone(),
        concurrency: inner.settings.concurrency,
        atom_poll_secs: inner.settings.atom_poll_secs,
        metadata_poll_secs: inner.settings.metadata_poll_secs,
        check_on_start: inner.settings.check_on_start,
        has_token: state.token(&inner).is_some(),
    })
}

#[tauri::command]
fn set_config(state: State<'_, AppState>, config: ConfigDto) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    // Validate the whole candidate first: a bad value rejects the save
    // with the reason, instead of being silently clamped or ignored.
    let mut candidate = inner.settings.clone();
    candidate.download_dir = config.download_dir.trim().to_string();
    candidate.concurrency = config.concurrency;
    candidate.atom_poll_secs = config.atom_poll_secs;
    candidate.metadata_poll_secs = config.metadata_poll_secs;
    candidate.check_on_start = config.check_on_start;
    candidate.validate()?;
    inner.settings.download_dir = candidate.download_dir;
    inner.settings.concurrency = candidate.concurrency;
    inner.settings.atom_poll_secs = candidate.atom_poll_secs;
    inner.settings.metadata_poll_secs = candidate.metadata_poll_secs;
    inner.settings.check_on_start = candidate.check_on_start;
    state.save_settings(&inner)
}

#[tauri::command]
fn set_token(state: State<'_, AppState>, token: String) -> Result<(), String> {
    let protected = dpapi::protect_token(token.trim())?;
    let mut inner = state.inner.lock().unwrap();
    inner.settings.token_protected = protected;
    log(
        &mut inner,
        "Personal access token stored (DPAPI-protected)".into(),
    );
    state.save_settings(&inner)
}

#[tauri::command]
fn clear_token(state: State<'_, AppState>) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    inner.settings.token_protected.clear();
    state.save_settings(&inner)
}

#[tauri::command]
fn get_activity(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let inner = state.inner.lock().unwrap();
    Ok(inner.activity.clone())
}

// ---------- The Librarian ----------

/// One Librarian pass: Atom checks for everything (free, in parallel),
/// downloads for standing orders with a newer release, and a real
/// metadata refresh when due (policy-governed, quota reported). Network
/// work runs on snapshots without the state lock; results merge back
/// under it. Returns the activity lines the pass produced.
fn librarian_pass(state: &AppState) -> Vec<String> {
    let before = {
        let inner = state.inner.lock().unwrap();
        inner.activity.len()
    };
    let now = now_unix();
    let fetcher = CurlFetcher::default();
    let (entries, settings_snapshot, token) = {
        let inner = state.inner.lock().unwrap();
        (
            inner.catalog.entries.clone(),
            inner.settings.clone(),
            state.token(&inner),
        )
    };
    // Atom checks in parallel; standing orders with a newer tag queue up.
    let results = check_entries_parallel(
        &fetcher,
        &entries,
        now,
        settings_snapshot.concurrency as usize,
    );
    let mut to_download: Vec<(RepoRef, String)> = Vec::new();
    {
        let mut inner = state.inner.lock().unwrap();
        for r in &results {
            if let Some(entry) = inner.catalog.get_mut(&r.repo) {
                entry.latest_tag = r.entry.latest_tag.clone();
                entry.last_release_check = r.entry.last_release_check;
            }
            if let Ok(out) = &r.outcome {
                if out.changed {
                    log(&mut inner, format!("{}: new release {}", r.repo, out.latest_tag));
                }
                if r.entry.auto_download
                    && !out.latest_tag.is_empty()
                    && out.latest_tag != r.entry.installed_tag
                {
                    to_download.push((r.repo.clone(), out.latest_tag.clone()));
                }
            }
        }
        inner.last_atom_run = now;
        let _ = state.save_catalog(&inner);
    }
    // Standing-order downloads: fetch each release's assets unlocked,
    // then enqueue under the lock.
    for (repo, tag) in to_download {
        match fetch_release_assets(&fetcher, &repo, &tag, token.as_deref()) {
            Ok((assets, _)) => {
                let mut inner = state.inner.lock().unwrap();
                let entry = match inner.catalog.get(&repo) {
                    Some(e) => e.clone(),
                    None => continue,
                };
                let root = state.downloads_root(&inner.settings);
                if enqueue_download(&mut inner.queue, &entry, &tag, &assets, &root).is_ok() {
                    log(&mut inner, format!("Standing order: queued {repo} {tag}"));
                }
            }
            Err(msg) => {
                let mut inner = state.inner.lock().unwrap();
                log(&mut inner, format!("{repo}: {msg}"));
            }
        }
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if !inner.queue.jobs.is_empty() {
            let _ = run_queue_locked(state, &mut inner);
        }
    }
    // Metadata refresh when due — the policy decides what is stale and
    // every call is accounted in the log line.
    let metadata_due = {
        let inner = state.inner.lock().unwrap();
        now.saturating_sub(inner.last_metadata_run) >= inner.settings.metadata_poll_secs
    };
    if metadata_due {
        let policy = CachePolicy::for_auth(auth_for_token(token.as_deref()));
        let (mut updated, mut spent) = (0usize, 0u32);
        let mut refreshed: Vec<RepoEntry> = Vec::new();
        let results = refresh_entries_parallel(
            &fetcher,
            &entries,
            &policy,
            token.as_deref(),
            now,
            false,
            settings_snapshot.concurrency as usize,
        );
        for r in results {
            match r.outcome {
                Ok(RefreshOutcome::Updated { quota_spent }) => {
                    updated += 1;
                    spent += quota_spent;
                    refreshed.push(r.entry);
                }
                Ok(RefreshOutcome::NotModified { quota_spent }) => {
                    spent += quota_spent;
                    refreshed.push(r.entry);
                }
                _ => {}
            }
        }
        let mut inner = state.inner.lock().unwrap();
        for entry in &refreshed {
            if let Some(stored) = inner.catalog.get_mut(&entry.repo) {
                apply_refreshed(stored, entry);
            }
        }
        inner.last_metadata_run = now;
        log(
            &mut inner,
            format!(
                "Librarian metadata refresh: {updated} updated, {spent} API call(s) of {} this hour",
                policy.auth.hourly_rest_limit()
            ),
        );
        let _ = state.save_catalog(&inner);
    }
    let inner = state.inner.lock().unwrap();
    let _ = state.save_queue(&inner);
    inner.activity[before..].to_vec()
}

#[tauri::command]
fn librarian_now(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(librarian_pass(&state))
}

fn scheduler_loop(handle: AppHandle) {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let due = {
            let state = handle.state::<AppState>();
            let inner = state.inner.lock().unwrap();
            inner.settings.atom_poll_secs > 0
                && now_unix().saturating_sub(inner.last_atom_run) >= inner.settings.atom_poll_secs
                && !inner.catalog.entries.is_empty()
        };
        if due {
            let state = handle.state::<AppState>();
            // A panicking pass must never kill the scheduler thread —
            // without it, the Librarian would silently stop forever.
            let lines = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                librarian_pass(&state)
            })) {
                Ok(lines) => lines,
                Err(_) => {
                    let mut inner = state.inner.lock().unwrap();
                    log(
                        &mut inner,
                        "Librarian pass hit an internal error; it will retry on the next interval.".into(),
                    );
                    let lines: Vec<String> = inner.activity.iter().rev().take(1).cloned().collect();
                    lines
                }
            };
            if !lines.is_empty() {
                let _ = handle.emit("librarian-activity", lines);
            }
        }
    }
}

fn main() {
    let exe_dir = std::env::current_exe()
        .map(|p| p.parent().map(|d| d.to_path_buf()).unwrap_or_default())
        .unwrap_or_default();
    let data_dir = exe_dir.join("data");
    let state = AppState {
        inner: Mutex::new(Inner {
            catalog: Catalog::default(),
            settings: Settings::default(),
            queue: Queue::default(),
            activity: Vec::new(),
            last_atom_run: 0,
            last_metadata_run: 0,
        }),
        data_dir,
        exe_dir,
    };
    // Load persisted state (paths honour the same env overrides as the CLI).
    // Loads recover from a backup when a file is corrupt; a load that
    // still fails is said out loud in the activity log, never swallowed.
    {
        let mut inner = state.inner.lock().unwrap();
        match Catalog::load(&state.catalog_path()) {
            Ok(c) => inner.catalog = c,
            Err(e) => log(
                &mut inner,
                format!("Catalogue could not be loaded ({e}) — starting empty; the old file stays on disk."),
            ),
        }
        match Settings::load(&state.settings_path()) {
            Ok(s) => inner.settings = s,
            Err(e) => log(
                &mut inner,
                format!("Settings could not be loaded ({e}) — using defaults."),
            ),
        }
        match Queue::load(&state.queue_path()) {
            Ok(q) => inner.queue = q,
            Err(e) => log(
                &mut inner,
                format!("Queue could not be loaded ({e}) — starting empty."),
            ),
        }
        log(&mut inner, "The Stacks opened".into());
    }
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
            // Optional check-on-start: one Librarian pass shortly after
            // launch, on its own thread so the window opens instantly.
            {
                let state = handle.state::<AppState>();
                let (on, has_volumes) = {
                    let inner = state.inner.lock().unwrap();
                    (
                        inner.settings.check_on_start,
                        !inner.catalog.entries.is_empty(),
                    )
                };
                if on && has_volumes {
                    let handle2 = handle.clone();
                    std::thread::spawn(move || {
                        let state = handle2.state::<AppState>();
                        let lines = librarian_pass(&state);
                        if !lines.is_empty() {
                            let _ = handle2.emit("librarian-activity", lines);
                        }
                    });
                }
            }
            std::thread::spawn(move || scheduler_loop(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            overview,
            find_entries,
            facets,
            add_repo,
            remove_repo,
            import_apply,
            import_user,
            open_repo_page,
            open_download_folder,
            saved_list,
            saved_save,
            saved_delete,
            list_shelves,
            shelf_add,
            shelf_remove,
            set_rules,
            set_auto,
            check_all,
            refresh_all,
            release_history,
            release_assets,
            download_repo,
            queue_list,
            queue_run,
            queue_pause,
            queue_resume,
            queue_cancel,
            queue_retry,
            queue_clear,
            get_view,
            set_view,
            get_config,
            set_config,
            set_token,
            clear_token,
            get_activity,
            librarian_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RepoStacks");
}
