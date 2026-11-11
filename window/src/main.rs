// RepoStacks window — a thin Tauri shell over repostacks_core. Every
// capability is a core operation first; these commands only adapt them.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dpapi;
mod single_instance;

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::cache_policy::CachePolicy;
use repostacks_core::catalog::Catalog;
use repostacks_core::catalog::RepoEntry;
use repostacks_core::download::{
    enqueue_download, plan_download, run_pending, suggest_asset_patterns,
};
use repostacks_core::http::{CurlDownloader, CurlFetcher, Fetcher};
use repostacks_core::net::{
    auth_for_token, check_entries_parallel, fetch_release_assets, refresh_entries_parallel,
    RefreshOutcome,
};
use repostacks_core::queue::{JobStatus, Queue};
use repostacks_core::search::{execute as search_execute, Query};
use repostacks_core::usage::{UsageKind, UsageLedger};
use repostacks_core::user_import::{
    apply_user_import, fetch_org_repos, fetch_user_repos, Archived, Forks, Ownership,
    UserRepoFilter, Visibility,
};
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
    usage: UsageLedger,
    activity: Vec<String>,
    last_atom_run: u64,
    last_metadata_run: u64,
    /// Last (bytes_done, instant) seen per job, for live speed/ETA.
    speed_samples: std::collections::HashMap<u64, (u64, std::time::Instant)>,
    /// True while the background download worker is alive.
    download_worker: bool,
    /// Downloads asked for but not yet planned: (repo, tag if pinned).
    pending_downloads: Vec<(RepoRef, Option<String>)>,
    /// A "run the queue" request waiting for the worker.
    pending_run: bool,
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
    fn usage_path(&self) -> PathBuf {
        env_or("REPOSTACKS_USAGE", self.data_dir.join("usage.json"))
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
    fn save_usage(&self, inner: &Inner) -> Result<(), String> {
        inner
            .usage
            .save(&self.usage_path())
            .map_err(|e| format!("save usage ledger: {e}"))
    }
    /// Record counted REST calls in the standing local meter. Saving
    /// is best-effort here because the network call already happened;
    /// a failed save is said in the activity log instead of failing
    /// the operation the user asked for.
    fn record_usage(&self, inner: &mut Inner, kind: UsageKind, calls: u32) {
        if calls == 0 {
            return;
        }
        inner.usage.record(kind, calls, now_unix());
        if let Err(e) = self.save_usage(inner) {
            log(
                inner,
                format!("API usage meter could not be saved ({e}); its counts may be incomplete."),
            );
        }
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
    stored.forks = refreshed.forks;
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
    forks: u64,
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
        forks: e.forks,
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
struct UsageKindDto {
    name: String,
    label: String,
    hour: u32,
    day: u32,
    lifetime: u64,
}

#[derive(Serialize)]
struct UsageDto {
    mode: String,
    hourly_limit: u32,
    hour_spent: u32,
    hour_percent: f64,
    hour_reset_unix: u64,
    day_spent: u32,
    lifetime_spent: u64,
    last_spent_unix: u64,
    kinds: Vec<UsageKindDto>,
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
    /// Live transfer rate (bytes/sec) and estimated seconds remaining,
    /// derived from successive queue polls; 0 when unknown.
    speed_bps: u64,
    eta_secs: u64,
}

// ---------- Catalogue commands ----------

#[tauri::command]
fn overview(state: State<'_, AppState>, app: AppHandle) -> Result<OverviewDto, String> {
    update_presence(&app);
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

/// Put many volumes on a shelf at once; returns how many placements
/// were new.
#[tauri::command]
fn bulk_shelf_add(
    state: State<'_, AppState>,
    shelf: String,
    repos: Vec<String>,
) -> Result<usize, String> {
    let shelf = shelf.trim().to_string();
    if shelf.is_empty() {
        return Err("A shelf needs a name.".into());
    }
    let mut inner = state.inner.lock().unwrap();
    let mut placed = 0;
    for name in &repos {
        if let Ok(repo) = RepoRef::parse(name) {
            if inner.catalog.add_to_shelf(&shelf, &repo) {
                placed += 1;
            }
        }
    }
    if placed > 0 {
        log(
            &mut inner,
            format!("Put {placed} volume(s) on shelf '{shelf}'"),
        );
        state.save_catalog(&inner)?;
    }
    Ok(placed)
}

/// Remove many volumes at once (the UI arms this like any removal).
#[tauri::command]
fn bulk_remove(state: State<'_, AppState>, repos: Vec<String>) -> Result<usize, String> {
    let mut inner = state.inner.lock().unwrap();
    let mut removed = 0;
    for name in &repos {
        if let Ok(repo) = RepoRef::parse(name) {
            if inner.catalog.get(&repo).is_some() {
                inner.catalog.remove(&repo);
                removed += 1;
            }
        }
    }
    if removed > 0 {
        log(&mut inner, format!("Removed {removed} volume(s)"));
        state.save_catalog(&inner)?;
    }
    Ok(removed)
}

/// Write the full catalogue JSON into the downloads folder as
/// `repostacks-catalogue-export.json` — the window's backup, matching
/// the CLI's `export --json`. Returns the file path.
#[tauri::command]
fn export_catalog(state: State<'_, AppState>) -> Result<String, String> {
    let mut inner = state.inner.lock().unwrap();
    let root = state.downloads_root(&inner.settings);
    std::fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;
    let file = root.join("repostacks-catalogue-export.json");
    std::fs::write(&file, inner.catalog.to_json().to_string_pretty())
        .map_err(|e| format!("write {}: {e}", file.display()))?;
    let count = inner.catalog.entries.len();
    log(
        &mut inner,
        format!("Exported {count} volume(s) to {}", file.display()),
    );
    Ok(file.display().to_string())
}

/// Replace the catalogue with a backup written by export_catalog (the
/// same fixed file in the downloads folder). The current catalogue is
/// kept as the save system's `.bak` when the import is written.
#[tauri::command]
fn import_catalog(state: State<'_, AppState>) -> Result<usize, String> {
    let mut inner = state.inner.lock().unwrap();
    let root = state.downloads_root(&inner.settings);
    let file = root.join("repostacks-catalogue-export.json");
    let text = std::fs::read_to_string(&file).map_err(|e| {
        format!(
            "read {}: {e} — export a catalogue there first",
            file.display()
        )
    })?;
    let parsed = repostacks_core::json::parse(&text)
        .map_err(|e| format!("{} did not parse: {e}", file.display()))?;
    let cat = Catalog::from_json(&parsed)
        .map_err(|e| format!("{} is not a usable catalogue backup: {e}", file.display()))?;
    let count = cat.entries.len();
    inner.catalog = cat;
    log(
        &mut inner,
        format!("Imported a catalogue backup: {count} volume(s)"),
    );
    state.save_catalog(&inner)?;
    Ok(count)
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
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
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
    #[serde(default)]
    org: bool,
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
fn import_user(
    state: State<'_, AppState>,
    input: UserImportDto,
) -> Result<serde_json::Value, String> {
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
    let (repos, spent) = if input.org {
        fetch_org_repos(&fetcher, &input.username, token.as_deref())?
    } else {
        fetch_user_repos(&fetcher, &input.username, token.as_deref())?
    };
    {
        let mut inner = state.inner.lock().unwrap();
        state.record_usage(&mut inner, UsageKind::Imports, spent);
    }
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
fn check_all(state: State<'_, AppState>, app: AppHandle) -> Result<Vec<CheckDto>, String> {
    let out = check_impl(&state, None)?;
    update_presence(&app);
    Ok(out)
}

/// Check a chosen subset (the volume page's single-volume check, the
/// catalogue's bulk check). Same snapshot/merge discipline as check_all.
#[tauri::command]
fn check_repos(
    state: State<'_, AppState>,
    app: AppHandle,
    repos: Vec<String>,
) -> Result<Vec<CheckDto>, String> {
    let want: Vec<RepoRef> = repos
        .iter()
        .filter_map(|r| RepoRef::parse(r).ok())
        .collect();
    let out = check_impl(&state, Some(&want))?;
    update_presence(&app);
    Ok(out)
}

fn check_impl(state: &AppState, only: Option<&[RepoRef]>) -> Result<Vec<CheckDto>, String> {
    // Snapshot, then run the checks in parallel WITHOUT holding the
    // state lock — the rest of the app stays responsive while the
    // network works. Results merge back under the lock at the end.
    let (entries, concurrency, token) = {
        let inner = state.inner.lock().unwrap();
        let entries = match only {
            Some(want) => inner
                .catalog
                .entries
                .iter()
                .filter(|e| want.contains(&e.repo))
                .cloned()
                .collect(),
            None => inner.catalog.entries.clone(),
        };
        (
            entries,
            inner.settings.concurrency as usize,
            state.token(&inner),
        )
    };
    let fetcher = CurlFetcher::default();
    let now = now_unix();
    let results = check_entries_parallel(&fetcher, &entries, now, concurrency, token.as_deref());
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
                    log(
                        &mut inner,
                        format!("{}: new release {}", r.repo, o.latest_tag),
                    );
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
    let fallback_spent: u32 = results
        .iter()
        .filter_map(|r| r.outcome.as_ref().ok())
        .map(|o| o.quota_spent)
        .sum();
    state.record_usage(&mut inner, UsageKind::CheckFallback, fallback_spent);
    let quota_note = if fallback_spent == 0 {
        "0 API calls".to_string()
    } else {
        format!("{fallback_spent} API call(s) — feed-less repos used the fallback")
    };
    if only.is_none() {
        inner.last_atom_run = now;
        log(
            &mut inner,
            format!("Checked all volumes via Atom feeds ({quota_note})"),
        );
    } else {
        log(
            &mut inner,
            format!(
                "Checked {} volume(s) via Atom feeds ({quota_note})",
                entries.len()
            ),
        );
    }
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
    state.record_usage(&mut inner, UsageKind::Metadata, spent);
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
        .map(|n| serde_json::json!({ "tag": n.tag, "title": n.title, "updated": n.updated, "body": n.body }))
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
    {
        let mut inner = state.inner.lock().unwrap();
        state.record_usage(&mut inner, UsageKind::Assets, spent);
    }
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

/// Ask the background download worker to pick up whatever is
/// pending (specific download requests and/or runnable queue jobs).
/// Downloads used to run under the state lock, which froze every
/// other command for the length of a transfer; the worker plans and
/// runs on snapshots instead, and merges its results back in brief
/// locked sections. Spawns the worker when it is not already alive;
/// the worker loops until nothing is pending, so a request made
/// mid-run is picked up rather than lost.
fn ensure_download_worker(state: &AppState, app: &AppHandle) {
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.download_worker {
            return;
        }
        inner.download_worker = true;
    }
    let app = app.clone();
    std::thread::spawn(move || download_worker_loop(&app));
}

fn download_worker_loop(app: &AppHandle) {
    // A panicking worker must not wedge downloads for the rest of
    // the session: reset the flag, re-queue anything it had marked
    // Downloading, and say so in the log.
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| download_worker_run(app)));
    if result.is_err() {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        inner.download_worker = false;
        for job in inner.queue.jobs.iter_mut() {
            if job.status == JobStatus::Downloading {
                job.status = JobStatus::Queued;
            }
        }
        log(
            &mut inner,
            "Download worker stopped after an internal error; new downloads will restart it."
                .into(),
        );
        let _ = state.save_queue(&inner);
    }
}

fn download_worker_run(app: &AppHandle) {
    let state = app.state::<AppState>();
    loop {
        // Drain requests and snapshot everything the run needs.
        let (requests, want_run, mut catalog, settings, token) = {
            let mut inner = state.inner.lock().unwrap();
            let token = state.token(&inner);
            (
                std::mem::take(&mut inner.pending_downloads),
                std::mem::take(&mut inner.pending_run),
                inner.catalog.clone(),
                inner.settings.clone(),
                token,
            )
        };
        let root = state.downloads_root(&settings);
        let mut work_queue = {
            let inner = state.inner.lock().unwrap();
            inner.queue.clone()
        };
        // Plan requested downloads without holding the lock (the
        // asset fetch is a network call per repo).
        let fetcher = CurlFetcher::default();
        let mut notes: Vec<String> = Vec::new();
        for (repo, tag) in &requests {
            let Some(entry) = catalog.get(repo).cloned() else {
                notes.push(format!("{repo}: not in the catalogue — download skipped"));
                continue;
            };
            let tag = tag
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| entry.latest_tag.clone());
            if tag.is_empty() {
                notes.push(format!("{repo}: no tag known — check for releases first"));
                continue;
            }
            match fetch_release_assets(&fetcher, repo, &tag, token.as_deref()) {
                Ok((assets, spent)) => {
                    {
                        let mut inner = state.inner.lock().unwrap();
                        state.record_usage(&mut inner, UsageKind::Assets, spent);
                    }
                    match enqueue_download(&mut work_queue, &entry, &tag, &assets, &root) {
                        Ok(id) => notes.push(format!("Queued #{id}: {repo} {tag}")),
                        Err(e) => notes.push(format!("{repo}: {e}")),
                    }
                }
                Err(e) => notes.push(format!("{repo}: {e}")),
            }
        }
        let has_runnable = work_queue
            .jobs
            .iter()
            .any(|j| j.status == JobStatus::Queued);
        if !want_run && !has_runnable {
            // Nothing to transfer. File the planning notes, then
            // stop — unless a request raced in while planning.
            let mut inner = state.inner.lock().unwrap();
            for n in notes {
                log(&mut inner, n);
            }
            // A retry pressed mid-run re-queues a job without
            // spawning a worker (this one was alive) — do not stop
            // while runnable work is sitting in the live queue.
            let queued_waiting = inner
                .queue
                .jobs
                .iter()
                .any(|j| j.status == JobStatus::Queued);
            if inner.pending_downloads.is_empty() && !inner.pending_run && !queued_waiting {
                inner.download_worker = false;
                return;
            }
            continue;
        }
        // Mark the batch Downloading in the live queue, so the
        // Acquisitions view can show live .part progress while the
        // run works on its snapshot.
        {
            let mut inner = state.inner.lock().unwrap();
            for job in work_queue
                .jobs
                .iter()
                .filter(|j| j.status == JobStatus::Queued)
            {
                if let Some(live) = inner.queue.jobs.iter_mut().find(|j| j.id == job.id) {
                    if live.status == JobStatus::Queued {
                        live.status = JobStatus::Downloading;
                    }
                }
            }
        }
        let downloader = CurlDownloader::default();
        let outcomes = run_pending(
            &mut work_queue,
            &mut catalog,
            &downloader,
            &root,
            token.as_deref(),
            settings.concurrency as usize,
        );
        for o in &outcomes {
            if o.status == JobStatus::Done {
                notes.push(format!("Downloaded → {}", o.final_path.display()));
            } else {
                notes.push(format!("Download failed: {}", o.error));
            }
        }
        // Merge back. The worker's job rows win for the ids it ran;
        // jobs added while it worked survive untouched. Only
        // installed_tag flows back into the live catalogue, so
        // edits made during the run are preserved. Pause/cancel
        // pressed mid-run apply to the live rows and are then
        // superseded by the finished run's truth for its own batch.
        {
            let mut inner = state.inner.lock().unwrap();
            for job in work_queue.jobs {
                if let Some(live) = inner.queue.jobs.iter_mut().find(|j| j.id == job.id) {
                    *live = job;
                } else {
                    inner.queue.jobs.push(job);
                }
            }
            for entry in &catalog.entries {
                if !entry.installed_tag.is_empty() {
                    if let Some(live) = inner.catalog.get_mut(&entry.repo) {
                        live.installed_tag = entry.installed_tag.clone();
                    }
                }
            }
            for n in notes {
                log(&mut inner, n);
            }
            let _ = state.save_queue(&inner);
            let _ = state.save_catalog(&inner);
        }
        update_presence(app);
    }
}

#[tauri::command]
fn download_repo(
    state: State<'_, AppState>,
    app: AppHandle,
    repo: String,
    tag: Option<String>,
) -> Result<String, String> {
    let repo_ref = RepoRef::parse(&repo).map_err(|e| e.0)?;
    {
        let mut inner = state.inner.lock().unwrap();
        let entry = inner
            .catalog
            .get(&repo_ref)
            .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
        let tag = tag
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| entry.latest_tag.clone());
        if tag.is_empty() {
            return Err(format!(
                "no tag known for {repo} — check for releases first"
            ));
        }
        log(&mut inner, format!("Download requested: {repo} {tag}"));
        inner.pending_downloads.push((repo_ref, Some(tag)));
    }
    ensure_download_worker(&state, &app);
    Ok("Download running in the background — progress is in Acquisitions".into())
}

#[tauri::command]
fn queue_list(state: State<'_, AppState>) -> Result<Vec<JobDto>, String> {
    let mut inner = state.inner.lock().unwrap();
    let root = state.downloads_root(&inner.settings);
    // Snapshot the job fields first so the speed samples can be
    // updated while the DTOs are built.
    let raw: Vec<(
        u64,
        String,
        String,
        String,
        String,
        String,
        JobStatus,
        u64,
        u64,
        f64,
        String,
    )> = inner
        .queue
        .jobs
        .iter()
        .map(|j| {
            (
                j.id,
                j.repo.full_name(),
                j.repo.owner.clone(),
                j.repo.name.clone(),
                j.tag.clone(),
                j.asset_name.clone(),
                j.status,
                j.bytes_done,
                j.bytes_total,
                j.progress_percent(),
                j.error.clone(),
            )
        })
        .collect();
    let now_i = std::time::Instant::now();
    let mut out = Vec::with_capacity(raw.len());
    for (id, repo, owner, name, tag, asset_name, status, done, total, mut percent, error) in raw {
        // A Downloading job is being written by the background
        // worker; its live byte count is the .part file's size.
        let done = if status == JobStatus::Downloading {
            let part = root
                .join(format!("{owner}-{name}"))
                .join(&tag)
                .join(format!("{asset_name}.part"));
            let live = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if live > done {
                if total > 0 {
                    percent = (live as f64 * 100.0 / total as f64).min(100.0);
                }
                live
            } else {
                done
            }
        } else {
            done
        };
        let (mut speed_bps, mut eta_secs) = (0u64, 0u64);
        if status == JobStatus::Downloading {
            if let Some(&(prev_bytes, prev_t)) = inner.speed_samples.get(&id) {
                let dt = now_i.duration_since(prev_t).as_secs_f64();
                if dt >= 0.5 && done > prev_bytes {
                    speed_bps = ((done - prev_bytes) as f64 / dt) as u64;
                    if speed_bps > 0 && total > done {
                        eta_secs = (total - done) / speed_bps;
                    }
                }
            }
            inner.speed_samples.insert(id, (done, now_i));
        } else {
            inner.speed_samples.remove(&id);
        }
        out.push(JobDto {
            id,
            repo,
            tag,
            asset_name,
            status: status.label().to_string(),
            bytes_done: done,
            bytes_total: total,
            percent,
            error,
            speed_bps,
            eta_secs,
        });
    }
    Ok(out)
}

#[tauri::command]
fn queue_run(state: State<'_, AppState>, app: AppHandle) -> Result<Vec<String>, String> {
    {
        let mut inner = state.inner.lock().unwrap();
        inner.pending_run = true;
    }
    ensure_download_worker(&state, &app);
    Ok(vec![
        "Downloads running in the background — progress below, live.".into(),
    ])
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
fn queue_retry(state: State<'_, AppState>, app: AppHandle, id: u64) -> Result<(), String> {
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.queue.retry(id) {
            state.save_queue(&inner)?;
        } else {
            return Err(format!("job {id} is not failed — only failed jobs retry"));
        }
    }
    // A retried job runs right away on the background worker.
    ensure_download_worker(&state, &app);
    Ok(())
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

#[tauri::command]
fn get_usage(state: State<'_, AppState>) -> Result<UsageDto, String> {
    let inner = state.inner.lock().unwrap();
    let token = state.token(&inner);
    let snap = inner
        .usage
        .snapshot(auth_for_token(token.as_deref()), now_unix());
    Ok(UsageDto {
        mode: if token.is_some() {
            "token".into()
        } else {
            "anonymous".into()
        },
        hourly_limit: snap.hourly_limit,
        hour_spent: snap.hour_spent,
        hour_percent: snap.hour_percent(),
        hour_reset_unix: snap.hour_reset_unix,
        day_spent: snap.day_spent,
        lifetime_spent: snap.lifetime_spent,
        last_spent_unix: snap.last_spent_unix,
        kinds: UsageKind::all()
            .into_iter()
            .map(|kind| UsageKindDto {
                name: kind.name().into(),
                label: kind.label().into(),
                hour: snap.hour_by_kind.get(&kind).copied().unwrap_or(0),
                day: snap.day_by_kind.get(&kind).copied().unwrap_or(0),
                lifetime: snap.lifetime_by_kind.get(&kind).copied().unwrap_or(0),
            })
            .collect(),
    })
}

// ---------- The Librarian ----------

/// One Librarian pass: Atom checks for everything (free, in parallel),
/// downloads for standing orders with a newer release, and a real
/// metadata refresh when due (policy-governed, quota reported). Network
/// work runs on snapshots without the state lock; results merge back
/// under it. Returns the activity lines the pass produced.
fn librarian_pass(state: &AppState, app: &AppHandle) -> Vec<String> {
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
        token.as_deref(),
    );
    let mut to_download: Vec<(RepoRef, String)> = Vec::new();
    {
        let mut inner = state.inner.lock().unwrap();
        state.record_usage(
            &mut inner,
            UsageKind::CheckFallback,
            results
                .iter()
                .filter_map(|r| r.outcome.as_ref().ok())
                .map(|o| o.quota_spent)
                .sum(),
        );
        for r in &results {
            if let Some(entry) = inner.catalog.get_mut(&r.repo) {
                entry.latest_tag = r.entry.latest_tag.clone();
                entry.last_release_check = r.entry.last_release_check;
            }
            if let Ok(out) = &r.outcome {
                if out.changed {
                    log(
                        &mut inner,
                        format!("{}: new release {}", r.repo, out.latest_tag),
                    );
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
            Ok((assets, spent)) => {
                let mut inner = state.inner.lock().unwrap();
                state.record_usage(&mut inner, UsageKind::Assets, spent);
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
    // Standing-order downloads run on the background worker too —
    // a Librarian pass must never freeze the window behind a long
    // transfer.
    if state
        .inner
        .lock()
        .unwrap()
        .queue
        .jobs
        .iter()
        .any(|j| j.status == JobStatus::Queued)
    {
        ensure_download_worker(state, app);
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
        state.record_usage(&mut inner, UsageKind::Metadata, spent);
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
fn librarian_now(state: State<'_, AppState>, app: AppHandle) -> Result<Vec<String>, String> {
    let lines = librarian_pass(&state, &app);
    update_presence(&app);
    Ok(lines)
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
                librarian_pass(&state, &handle)
            })) {
                Ok(lines) => lines,
                Err(_) => {
                    let mut inner = state.inner.lock().unwrap();
                    log(
                        &mut inner,
                        "Librarian pass hit an internal error; it will retry on the next interval."
                            .into(),
                    );
                    let lines: Vec<String> = inner.activity.iter().rev().take(1).cloned().collect();
                    lines
                }
            };
            update_presence(&handle);
            if !lines.is_empty() {
                let _ = handle.emit("librarian-activity", lines);
            }
        }
    }
}

/// Is this folder usable for the library's data? (Creates it and
/// proves a write + remove work.)
fn dir_writable(dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".repostacks-probe");
    std::fs::write(&probe, b"ok")
        .and_then(|_| std::fs::remove_file(&probe))
        .is_ok()
}

/// Where the library's data lives: beside the exe when that folder is
/// writable (the portable story); when it is not — an installed copy
/// under Program Files — fall back to %LOCALAPPDATA%\RepoStacks so
/// saves cannot fail just because of where the app was installed.
fn choose_data_dir(exe_dir: &std::path::Path) -> PathBuf {
    let beside = exe_dir.join("data");
    if dir_writable(&beside) {
        return beside;
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let fallback = PathBuf::from(local).join("RepoStacks").join("data");
        if dir_writable(&fallback) {
            return fallback;
        }
    }
    beside
}

/// Refresh the tray tooltip and the window title with the number of
/// volumes that have an update waiting.
fn update_presence(app: &AppHandle) {
    let count = {
        let state = app.state::<AppState>();
        let inner = state.inner.lock().unwrap();
        inner
            .catalog
            .entries
            .iter()
            .filter(|e| e.update_available())
            .count()
    };
    let plural = if count == 1 { "" } else { "s" };
    let title = if count == 0 {
        "RepoStacks — The Stacks".to_string()
    } else {
        format!("RepoStacks — The Stacks ({count} update{plural})")
    };
    let tip = if count == 0 {
        "RepoStacks — The Stacks".to_string()
    } else {
        format!("RepoStacks — {count} update{plural} waiting")
    };
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&title);
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tip.as_str()));
    }
}

fn main() {
    // One instance per session: a second copy would share the data
    // folder and last-writer-wins could drop edits. Step aside with
    // an explanation before touching any state.
    let _instance = match single_instance::acquire() {
        Some(guard) => guard,
        None => {
            single_instance::notify_already_running();
            return;
        }
    };
    let exe_dir = std::env::current_exe()
        .map(|p| p.parent().map(|d| d.to_path_buf()).unwrap_or_default())
        .unwrap_or_default();
    let data_dir = choose_data_dir(&exe_dir);
    let fell_back = data_dir != exe_dir.join("data");
    let state = AppState {
        inner: Mutex::new(Inner {
            catalog: Catalog::default(),
            settings: Settings::default(),
            queue: Queue::default(),
            usage: UsageLedger::default(),
            activity: Vec::new(),
            last_atom_run: 0,
            last_metadata_run: 0,
            speed_samples: std::collections::HashMap::new(),
            download_worker: false,
            pending_downloads: Vec::new(),
            pending_run: false,
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
        match UsageLedger::load(&state.usage_path()) {
            Ok(u) => inner.usage = u,
            Err(e) => log(
                &mut inner,
                format!("API usage ledger could not be loaded ({e}) — starting a fresh meter."),
            ),
        }
        log(&mut inner, "The Stacks opened".into());
        if fell_back {
            log(
                &mut inner,
                format!(
                    "The program folder is not writable, so your library lives in {}",
                    state.data_dir.display()
                ),
            );
        }
    }
    tauri::Builder::default()
        .manage(state)
        .on_window_event(|window, event| {
            // Closing the window parks RepoStacks in the tray — the
            // Librarian keeps its schedule there. Quit lives in the
            // tray menu.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            // System tray: presence with an updates count, Open /
            // Check now / Quit.
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                let open_item =
                    MenuItem::with_id(app, "tray_open", "Open RepoStacks", true, None::<&str>)
                        .expect("tray menu");
                let check_item =
                    MenuItem::with_id(app, "tray_check", "Check for updates", true, None::<&str>)
                        .expect("tray menu");
                let quit_item = MenuItem::with_id(app, "tray_quit", "Quit", true, None::<&str>)
                    .expect("tray menu");
                let menu = Menu::with_items(app, &[&open_item, &check_item, &quit_item])
                    .expect("tray menu");
                let mut builder = TrayIconBuilder::with_id("main")
                    .menu(&menu)
                    .tooltip("RepoStacks — The Stacks")
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "tray_open" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "tray_check" => {
                            let app = app.clone();
                            std::thread::spawn(move || {
                                let state = app.state::<AppState>();
                                if check_impl(&state, None).is_ok() {
                                    update_presence(&app);
                                }
                            });
                        }
                        "tray_quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            let app = tray.app_handle();
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                    });
                if let Some(icon) = app.default_window_icon() {
                    builder = builder.icon(icon.clone());
                }
                let _ = builder.build(app);
            }
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
                        let lines = librarian_pass(&state, &handle2);
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
            check_repos,
            bulk_shelf_add,
            bulk_remove,
            export_catalog,
            import_catalog,
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
            get_usage,
            librarian_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running RepoStacks");
}
