// RepoStacks window — a thin Tauri shell over repostacks_core. Every
// capability is a core operation first; these commands only adapt them.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dpapi;

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::cache_policy::CachePolicy;
use repostacks_core::catalog::Catalog;
use repostacks_core::download::{enqueue_download, plan_download, run_pending};
use repostacks_core::http::{CurlDownloader, CurlFetcher, Fetcher};
use repostacks_core::net::{
    auth_for_token, check_releases, fetch_release_assets, refresh_metadata, RefreshOutcome,
};
use repostacks_core::queue::{JobStatus, Queue};
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
    let hits = search_execute(&inner.catalog, &q);
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
    let mut inner = state.inner.lock().unwrap();
    let fetcher = CurlFetcher::default();
    let now = now_unix();
    let repos: Vec<RepoRef> = inner
        .catalog
        .entries
        .iter()
        .map(|e| e.repo.clone())
        .collect();
    let mut out = Vec::new();
    for repo in repos {
        let entry = inner.catalog.get_mut(&repo).expect("listed");
        match check_releases(&fetcher, entry, now) {
            Ok(o) => {
                if o.changed {
                    log(&mut inner, format!("{repo}: new release {}", o.latest_tag));
                }
                out.push(CheckDto {
                    repo: repo.full_name(),
                    latest: o.latest_tag,
                    changed: o.changed,
                    error: String::new(),
                });
            }
            Err(msg) => out.push(CheckDto {
                repo: repo.full_name(),
                latest: String::new(),
                changed: false,
                error: msg,
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
    let mut inner = state.inner.lock().unwrap();
    let token = state.token(&inner);
    let policy = CachePolicy::for_auth(auth_for_token(token.as_deref()));
    let fetcher = CurlFetcher::default();
    let now = now_unix();
    let repos: Vec<RepoRef> = inner
        .catalog
        .entries
        .iter()
        .map(|e| e.repo.clone())
        .collect();
    let (mut updated, mut unchanged, mut skipped, mut spent) = (0, 0, 0, 0u32);
    let mut errors = Vec::new();
    for repo in repos {
        let entry = inner.catalog.get_mut(&repo).expect("listed");
        match refresh_metadata(&fetcher, entry, &policy, token.as_deref(), now, force) {
            Ok(RefreshOutcome::Updated { quota_spent }) => {
                updated += 1;
                spent += quota_spent;
            }
            Ok(RefreshOutcome::NotModified { quota_spent }) => {
                unchanged += 1;
                spent += quota_spent;
            }
            Ok(RefreshOutcome::SkippedNotDue) => skipped += 1,
            Err(msg) => errors.push(format!("{repo}: {msg}")),
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
fn release_history(state: State<'_, AppState>, repo: String) -> Result<serde_json::Value, String> {
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
    let pick = plan_download(&entry, &tag, &assets, std::path::Path::new("."))
        .map(|p| p.asset_name)
        .unwrap_or_default();
    Ok(serde_json::json!({
        "assets": assets.iter().map(|a| serde_json::json!({
            "name": a.name, "size": a.size, "digest": a.digest,
        })).collect::<Vec<_>>(),
        "pick": pick,
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
        has_token: state.token(&inner).is_some(),
    })
}

#[tauri::command]
fn set_config(state: State<'_, AppState>, config: ConfigDto) -> Result<(), String> {
    let mut inner = state.inner.lock().unwrap();
    if !config.download_dir.trim().is_empty() {
        inner.settings.download_dir = config.download_dir.trim().to_string();
    }
    if config.concurrency >= 1 {
        inner.settings.concurrency = config.concurrency;
    }
    if config.atom_poll_secs >= 60 {
        inner.settings.atom_poll_secs = config.atom_poll_secs;
    }
    if config.metadata_poll_secs >= 60 {
        inner.settings.metadata_poll_secs = config.metadata_poll_secs;
    }
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

/// One Librarian pass: Atom checks for everything (free), downloads for
/// standing orders with a newer release, and a metadata refresh when due.
/// Returns the activity lines it produced.
fn librarian_pass(state: &AppState) -> Vec<String> {
    let before = {
        let inner = state.inner.lock().unwrap();
        inner.activity.len()
    };
    let mut inner = state.inner.lock().unwrap();
    let now = now_unix();
    let fetcher = CurlFetcher::default();
    let repos: Vec<RepoRef> = inner
        .catalog
        .entries
        .iter()
        .map(|e| e.repo.clone())
        .collect();
    let mut to_download: Vec<(RepoRef, String)> = Vec::new();
    for repo in &repos {
        let entry = inner.catalog.get_mut(repo).expect("listed");
        if let Ok(out) = check_releases(&fetcher, entry, now) {
            if out.changed {
                log(
                    &mut inner,
                    format!("{repo}: new release {}", out.latest_tag),
                );
            }
            if entry.auto_download
                && !out.latest_tag.is_empty()
                && out.latest_tag != entry.installed_tag
            {
                to_download.push((repo.clone(), out.latest_tag.clone()));
            }
        }
    }
    inner.last_atom_run = now;
    let _ = state.save_catalog(&inner);
    // Standing-order downloads.
    let token = state.token(&inner);
    for (repo, tag) in to_download {
        let entry = match inner.catalog.get(&repo) {
            Some(e) => e.clone(),
            None => continue,
        };
        match fetch_release_assets(&fetcher, &repo, &tag, token.as_deref()) {
            Ok((assets, _)) => {
                let root = state.downloads_root(&inner.settings);
                if enqueue_download(&mut inner.queue, &entry, &tag, &assets, &root).is_ok() {
                    log(&mut inner, format!("Standing order: queued {repo} {tag}"));
                }
            }
            Err(msg) => log(&mut inner, format!("{repo}: {msg}")),
        }
    }
    if !inner.queue.jobs.is_empty() {
        let _ = run_queue_locked(state, &mut inner);
    }
    // Metadata refresh when due.
    if now.saturating_sub(inner.last_metadata_run) >= inner.settings.metadata_poll_secs {
        inner.last_metadata_run = now;
        log(
            &mut inner,
            "Metadata refresh due — run Refresh from the Catalogue if wanted".into(),
        );
    }
    let _ = state.save_catalog(&inner);
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
            let lines = librarian_pass(&state);
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
    {
        let mut inner = state.inner.lock().unwrap();
        if let Ok(c) = Catalog::load(&state.catalog_path()) {
            inner.catalog = c;
        }
        if let Ok(s) = Settings::load(&state.settings_path()) {
            inner.settings = s;
        }
        if let Ok(q) = Queue::load(&state.queue_path()) {
            inner.queue = q;
        }
        log(&mut inner, "The Stacks opened".into());
    }
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            let handle = app.handle().clone();
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
