//! RepoStacks command line — every window capability is a command first.
//!
//! Data file: `REPOSTACKS_DATA` env var, else `data/catalog.json` beside the
//! working directory. The file carries `schema_version` and is written
//! atomically, so a crash never loses the catalogue.

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::cache_policy::CachePolicy;
use repostacks_core::catalog::{Catalog, RepoEntry};
use repostacks_core::download::{
    clean_downloads, enqueue_download, plan_download, run_pending, scan_downloads,
    suggest_asset_patterns,
};
use repostacks_core::glob_match::select_assets;
use repostacks_core::http::{hidden_command, CurlDownloader, CurlFetcher};
use repostacks_core::json::Json;
use repostacks_core::net::fetch_release_assets;
use repostacks_core::net::{
    auth_for_token, check_entries_parallel, check_releases, refresh_entries_parallel,
    RefreshOutcome,
};
use repostacks_core::queue::{JobStatus, Queue};
use repostacks_core::search::{execute as search_execute, Query};
use repostacks_core::usage::{UsageKind, UsageLedger};
use repostacks_core::user_import::{
    apply_user_import, fetch_org_repos, fetch_user_repos, Archived, Forks, Ownership,
    UserRepoFilter, Visibility,
};
use repostacks_core::view::{render, status_of, Column, Density, GroupBy, Layout, Settings, Theme};
use repostacks_core::RepoRef;
use std::path::PathBuf;
use std::process::ExitCode;

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn token() -> Option<String> {
    std::env::var("REPOSTACKS_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
}

fn queue_path() -> PathBuf {
    match std::env::var_os("REPOSTACKS_QUEUE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data").join("queue.json"),
    }
}

fn settings_path() -> PathBuf {
    match std::env::var_os("REPOSTACKS_SETTINGS") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data").join("settings.json"),
    }
}

fn usage_path() -> PathBuf {
    match std::env::var_os("REPOSTACKS_USAGE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data").join("usage.json"),
    }
}

/// Add counted REST calls to the persistent local usage ledger. The
/// meter is accounting for this app's own counted calls, not a live
/// GitHub quota reading. A ledger that cannot be loaded is left alone
/// rather than replaced.
fn record_usage(kind: UsageKind, calls: u32) {
    if calls == 0 {
        return;
    }
    if let Ok(mut ledger) = UsageLedger::load(&usage_path()) {
        ledger.record(kind, calls, now_unix());
        let _ = ledger.save(&usage_path());
    }
}

fn run_find(catalog: &Catalog, settings: &Settings, query_text: &str) -> Result<(), String> {
    let q = Query::parse(query_text);
    let hits = search_execute(catalog, &q, now_unix());
    print!("{}", render(&hits, catalog, &settings.view));
    println!("{} volume(s).", hits.len());
    Ok(())
}

/// An entry as JSON for `--json` output: the stored fields plus the
/// computed status and shelf membership the text views show.
fn entry_json(e: &RepoEntry, cat: &Catalog) -> Json {
    let mut obj = e.to_json();
    if let Json::Obj(pairs) = &mut obj {
        pairs.push(("status".into(), Json::Str(status_of(e).to_string())));
        pairs.push((
            "shelves".into(),
            Json::Arr(
                cat.shelves
                    .iter()
                    .filter(|(_, members)| members.contains(&e.repo.full_name()))
                    .map(|(name, _)| Json::Str(name.clone()))
                    .collect(),
            ),
        ));
    }
    obj
}

fn print_entries_json(entries: &[&RepoEntry], cat: &Catalog) {
    let arr = Json::Arr(entries.iter().map(|e| entry_json(e, cat)).collect());
    println!("{}", arr.to_string_pretty());
}

fn data_path() -> PathBuf {
    match std::env::var_os("REPOSTACKS_DATA") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data").join("catalog.json"),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("repostacks: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    // `--json` anywhere in the line switches the data commands (list,
    // find, updates, facets, queue list) to machine-readable output.
    let json_mode = args.iter().any(|a| a == "--json");
    let filtered: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let args = &filtered;
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let path = data_path();
    match cmd {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        "version" | "--version" => {
            println!("RepoStacks {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "add" => {
            let input = need(args, 1, "add <owner/repo | github URL>")?;
            let repo = RepoRef::parse(input).map_err(|e| e.0)?;
            let mut cat = Catalog::load(&path)?;
            if cat.add(repo.clone()) {
                cat.save(&path)
                    .map_err(|e| format!("save catalogue: {e}"))?;
                println!("Added {repo} to the catalogue.");
            } else {
                println!("{repo} is already in the catalogue.");
            }
            Ok(())
        }
        "import" => {
            // Bulk import (Add Volumes dialog, as an operation): one repo
            // per line from a file, or "-" for stdin. Optional shelf name
            // as the second argument puts every added volume on it.
            let file = need(args, 1, "import <file|-> [shelf]")?;
            let text = if file == "-" {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| format!("read stdin: {e}"))?;
                buf
            } else {
                std::fs::read_to_string(file).map_err(|e| format!("read import file: {e}"))?
            };
            let mut cat = Catalog::load(&path)?;
            let report = cat.import_list(&text);
            if let Some(shelf) = args.get(2) {
                for name in &report.added {
                    let repo = RepoRef::parse(name).map_err(|e| e.0)?;
                    cat.add_to_shelf(shelf, &repo);
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            println!(
                "Import preview: {} valid, {} duplicate(s), {} not valid.",
                report.added.len(),
                report.duplicates.len(),
                report.invalid.len()
            );
            for name in &report.added {
                println!("  + {name}");
            }
            for name in &report.duplicates {
                println!("  = {name} (already catalogued)");
            }
            for (line, why) in &report.invalid {
                println!("  ! {line} — {why}");
            }
            if let Some(shelf) = args.get(2) {
                if !report.added.is_empty() {
                    println!("Added volumes are on shelf '{shelf}'.");
                }
            }
            Ok(())
        }
        "import-user" | "import-org" => {
            // Import a GitHub user's (or organization's) projects, with
            // filters:
            //   --owned | --member            whose repos (default: all;
            //                                 users only — an org listing
            //                                 spans many owners)
            //   --forks | --sources           forks or originals (default: all)
            //   --with-archived | --only-archived  (default: archived left out)
            //   --public | --private          visibility (default: all)
            //   --dry-run                     preview the counts, add nothing
            let org = cmd == "import-org";
            let username = need(
                args,
                1,
                if org {
                    "import-org <github-org> [--forks|--sources] [--with-archived|--only-archived] [--public|--private] [--dry-run]"
                } else {
                    "import-user <github-username> [--owned|--member] [--forks|--sources] [--with-archived|--only-archived] [--public|--private] [--dry-run]"
                },
            )?;
            let mut filter = UserRepoFilter::default();
            let mut dry_run = false;
            for flag in &args[2..] {
                match flag.as_str() {
                    "--owned" | "--member" if org => {
                        return Err(
                            "--owned/--member do not apply to organizations: an org listing spans many owners"
                                .into(),
                        )
                    }
                    "--owned" => filter.ownership = Ownership::Owned,
                    "--member" => filter.ownership = Ownership::Member,
                    "--forks" => filter.forks = Forks::ForksOnly,
                    "--sources" => filter.forks = Forks::SourcesOnly,
                    "--with-archived" => filter.archived = Archived::Include,
                    "--only-archived" => filter.archived = Archived::Only,
                    "--public" => filter.visibility = Visibility::PublicOnly,
                    "--private" => filter.visibility = Visibility::PrivateOnly,
                    "--dry-run" => dry_run = true,
                    other => return Err(format!("unknown flag '{other}' for {cmd}")),
                }
            }
            let fetcher = CurlFetcher::default();
            let tok = token();
            let (repos, spent) = if org {
                fetch_org_repos(&fetcher, username, tok.as_deref())?
            } else {
                fetch_user_repos(&fetcher, username, tok.as_deref())?
            };
            record_usage(UsageKind::Imports, spent);
            let mut cat = Catalog::load(&path)?;
            let report =
                apply_user_import(&mut cat, &repos, &filter, username, now_unix(), dry_run);
            let matched = report.added.len() + report.duplicates.len();
            println!(
                "{username}: {} repo(s) listed, {} matched the filters, {} filtered out.",
                repos.len(),
                matched,
                repos.len() - matched
            );
            for name in &report.added {
                println!("  + {name}");
            }
            for name in &report.duplicates {
                println!("  = {name} (already catalogued)");
            }
            if dry_run {
                println!(
                    "Dry run: nothing added. {} API call(s) spent listing.",
                    spent
                );
            } else {
                if !report.added.is_empty() {
                    cat.save(&path)
                        .map_err(|e| format!("save catalogue: {e}"))?;
                }
                println!(
                    "Added {} volume(s). {} API call(s) spent listing.",
                    report.added.len(),
                    spent
                );
            }
            Ok(())
        }
        "export" => {
            // The catalogue in a portable form. Default: the importable
            // list (one owner/repo per line). --json: the full catalogue
            // JSON (the same schema as the data file). A path argument
            // writes to that file instead of stdout.
            let cat = Catalog::load(&path)?;
            let as_json = json_mode; // --json is the global flag, stripped upstream
            let dest = args.get(1).filter(|a| !a.starts_with("--"));
            let text = if as_json {
                cat.to_json().to_string_pretty()
            } else {
                cat.export_list()
            };
            match dest {
                Some(file) => {
                    std::fs::write(file, &text).map_err(|e| format!("write {file}: {e}"))?;
                    println!("Exported {} volume(s) to {file}.", cat.entries.len());
                }
                None => print!("{text}"),
            }
            Ok(())
        }
        "clean" => {
            // Download-folder housekeeping: folders of repos no longer
            // catalogued, and .part files no live job owns. Dry run
            // unless --yes — deleting downloads must be a decision.
            let yes = args.iter().any(|a| a == "--yes");
            let cat = Catalog::load(&path)?;
            let queue = Queue::load(&queue_path())?;
            let settings = Settings::load(&settings_path())?;
            let root = PathBuf::from(&settings.download_dir);
            let report = clean_downloads(&root, &cat, &queue, !yes)?;
            if report.stale_folders.is_empty() && report.orphan_parts.is_empty() {
                println!("Nothing to clean under {}.", root.display());
                return Ok(());
            }
            for f in &report.stale_folders {
                println!("  folder  {}", f.display());
            }
            for f in &report.orphan_parts {
                println!("  part    {}", f.display());
            }
            let mb = report.bytes as f64 / 1_000_000.0;
            if yes {
                println!(
                    "Removed {} item(s), {mb:.1} MB freed.",
                    report.stale_folders.len() + report.orphan_parts.len()
                );
            } else {
                println!(
                    "{} item(s), {mb:.1} MB. Dry run — pass --yes to delete.",
                    report.stale_folders.len() + report.orphan_parts.len()
                );
            }
            Ok(())
        }
        "remove" => {
            let repo = RepoRef::parse(need(args, 1, "remove <owner/repo>")?).map_err(|e| e.0)?;
            let mut cat = Catalog::load(&path)?;
            if cat.remove(&repo) {
                cat.save(&path)
                    .map_err(|e| format!("save catalogue: {e}"))?;
                println!("Removed {repo}.");
            } else {
                println!("{repo} is not in the catalogue.");
            }
            Ok(())
        }
        "list" => {
            let cat = Catalog::load(&path)?;
            if json_mode {
                let refs: Vec<&RepoEntry> = cat.entries.iter().collect();
                print_entries_json(&refs, &cat);
                return Ok(());
            }
            if cat.entries.is_empty() {
                println!("Your catalogue is empty. Add a volume: repostacks add owner/repo");
                return Ok(());
            }
            for e in &cat.entries {
                let installed = if e.is_installed() {
                    format!("  [installed {}]", e.installed_tag)
                } else {
                    String::new()
                };
                let update = if e.update_available() {
                    format!("  [update {} available]", e.latest_tag)
                } else {
                    String::new()
                };
                println!("{}{}{}", e.repo, installed, update);
            }
            Ok(())
        }
        "shelf" => {
            // shelf add <name> <owner/repo> | shelf list <name>
            match args.get(1).map(String::as_str) {
                Some("add") => {
                    let name = need(args, 2, "shelf add <name> <owner/repo>")?;
                    let repo = RepoRef::parse(need(args, 3, "shelf add <name> <owner/repo>")?)
                        .map_err(|e| e.0)?;
                    let mut cat = Catalog::load(&path)?;
                    if cat.add_to_shelf(name, &repo) {
                        cat.save(&path)
                            .map_err(|e| format!("save catalogue: {e}"))?;
                        println!("Put {repo} on shelf '{name}'.");
                    } else {
                        println!("{repo} is already on shelf '{name}' (or not catalogued).");
                    }
                    Ok(())
                }
                Some("list") => {
                    let name = need(args, 2, "shelf list <name>")?;
                    let cat = Catalog::load(&path)?;
                    let members = cat.shelf_members(name);
                    if members.is_empty() {
                        println!("Shelf '{name}' is empty.");
                    }
                    for e in members {
                        println!("{}", e.repo);
                    }
                    Ok(())
                }
                Some("update") => {
                    // Check every member (Atom, free, in parallel), then
                    // download the ones with something newer — the same
                    // per-volume flow as `update`, via the download arm.
                    let name = need(args, 2, "shelf update <name>")?;
                    let mut cat = Catalog::load(&path)?;
                    let members: Vec<RepoRef> = cat
                        .shelf_members(name)
                        .iter()
                        .map(|e| e.repo.clone())
                        .collect();
                    if members.is_empty() {
                        println!("Shelf '{name}' is empty.");
                        return Ok(());
                    }
                    let settings = Settings::load(&settings_path())?;
                    let fetcher = CurlFetcher::default();
                    let snapshots: Vec<RepoEntry> = members
                        .iter()
                        .filter_map(|r| cat.get(r).cloned())
                        .collect();
                    let tok = token();
                    let results = check_entries_parallel(
                        &fetcher,
                        &snapshots,
                        now_unix(),
                        settings.concurrency as usize,
                        tok.as_deref(),
                    );
                    record_usage(
                        UsageKind::CheckFallback,
                        results
                            .iter()
                            .filter_map(|r| r.outcome.as_ref().ok())
                            .map(|o| o.quota_spent)
                            .sum(),
                    );
                    let mut todo: Vec<RepoRef> = Vec::new();
                    for r in &results {
                        if let Some(slot) = cat.get_mut(&r.repo) {
                            slot.latest_tag = r.entry.latest_tag.clone();
                            slot.last_release_check = r.entry.last_release_check;
                        }
                        match &r.outcome {
                            Ok(_) => {
                                if !r.entry.latest_tag.is_empty()
                                    && r.entry.latest_tag != r.entry.installed_tag
                                {
                                    println!("{}: {} available.", r.repo, r.entry.latest_tag);
                                    todo.push(r.repo.clone());
                                } else {
                                    println!("{}: up to date.", r.repo);
                                }
                            }
                            Err(msg) => println!("{}: {msg}", r.repo),
                        }
                    }
                    cat.save(&path)
                        .map_err(|e| format!("save catalogue: {e}"))?;
                    let mut failed = 0;
                    for repo in todo {
                        let sub: Vec<String> = vec!["download".into(), repo.full_name()];
                        if run(&sub).is_err() {
                            failed += 1;
                        }
                    }
                    if failed > 0 {
                        Err(format!("{failed} shelf update(s) failed"))
                    } else {
                        Ok(())
                    }
                }
                _ => Err(
                    "usage: repostacks shelf add <name> <owner/repo> | shelf list <name> | shelf update <name>"
                        .into(),
                ),
            }
        }
        "find" => {
            if args.len() < 2 {
                return Err(
                    "usage: repostacks find <query> — e.g. find lang:rust stars:>1000 sort:-stars"
                        .into(),
                );
            }
            let cat = Catalog::load(&path)?;
            let settings = Settings::load(&settings_path())?;
            if json_mode {
                let q = Query::parse(&args[1..].join(" "));
                let hits = search_execute(&cat, &q, now_unix());
                print_entries_json(&hits, &cat);
                return Ok(());
            }
            run_find(&cat, &settings, &args[1..].join(" "))
        }
        "search" => {
            // Saved searches; a bare `search <words>` runs it as a find.
            let spath = settings_path();
            let mut settings = Settings::load(&spath)?;
            match args.get(1).map(String::as_str) {
                Some("save") => {
                    let name = need(args, 2, "search save <name> <query>")?;
                    if args.len() < 4 {
                        return Err("usage: repostacks search save <name> <query>".into());
                    }
                    let query = args[3..].join(" ");
                    settings
                        .saved_searches
                        .insert(name.to_string(), query.clone());
                    settings
                        .save(&spath)
                        .map_err(|e| format!("save settings: {e}"))?;
                    println!("Saved search '{name}': {query}");
                    Ok(())
                }
                Some("list") => {
                    if settings.saved_searches.is_empty() {
                        println!("No saved searches yet.");
                    }
                    for (name, q) in &settings.saved_searches {
                        println!("{name}: {q}");
                    }
                    Ok(())
                }
                Some("run") => {
                    let name = need(args, 2, "search run <name>")?;
                    let query = settings
                        .saved_searches
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("no saved search named '{name}'"))?;
                    let cat = Catalog::load(&path)?;
                    run_find(&cat, &settings, &query)
                }
                Some("delete") => {
                    let name = need(args, 2, "search delete <name>")?;
                    if settings.saved_searches.remove(name).is_some() {
                        settings
                            .save(&spath)
                            .map_err(|e| format!("save settings: {e}"))?;
                        println!("Deleted saved search '{name}'.");
                    } else {
                        println!("No saved search named '{name}'.");
                    }
                    Ok(())
                }
                Some(_) => {
                    let cat = Catalog::load(&path)?;
                    run_find(&cat, &settings, &args[1..].join(" "))
                }
                None => Err("usage: repostacks search save|list|run|delete …".into()),
            }
        }
        "view" => {
            let spath = settings_path();
            let mut settings = Settings::load(&spath)?;
            match args.get(1).map(String::as_str) {
                None => {
                    let v = &settings.view;
                    println!("layout:   {}", v.layout.name());
                    println!("theme:    {}", v.theme.name());
                    println!("density:  {}", v.density.name());
                    println!("group-by: {}", v.group_by.name());
                    println!(
                        "columns:  {}",
                        v.columns
                            .iter()
                            .map(|c| c.name())
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    println!("facets:   {}", if v.show_facets { "on" } else { "off" });
                    return Ok(());
                }
                Some("layout") => {
                    let name = need(args, 2, "view layout <ledger|cards|compact|diagram>")?;
                    settings.view.layout = Layout::from_name(name).ok_or_else(|| {
                        format!("unknown layout '{name}' (ledger, cards, compact, diagram)")
                    })?;
                }
                Some("theme") => {
                    let name = need(
                        args,
                        2,
                        "view theme <dark-ledger|paper-ledger|clean-sheet|terminal>",
                    )?;
                    settings.view.theme =
                        Theme::from_name(name).ok_or_else(|| format!("unknown theme '{name}'"))?;
                }
                Some("density") => {
                    let name = need(args, 2, "view density <comfortable|compact>")?;
                    settings.view.density = Density::from_name(name)
                        .ok_or_else(|| format!("unknown density '{name}'"))?;
                }
                Some("group-by") => {
                    let name = need(
                        args,
                        2,
                        "view group-by <none|shelf|language|license|status>",
                    )?;
                    settings.view.group_by = GroupBy::from_name(name)
                        .ok_or_else(|| format!("unknown group-by '{name}'"))?;
                }
                Some("columns") => {
                    let list = need(args, 2, "view columns <title,owner,stars,...>")?;
                    let cols: Result<Vec<Column>, String> = list
                        .split(',')
                        .map(|s| {
                            Column::from_name(s.trim())
                                .ok_or_else(|| format!("unknown column '{s}'"))
                        })
                        .collect();
                    let cols = cols?;
                    if cols.is_empty() {
                        return Err("columns may not be empty".into());
                    }
                    settings.view.columns = cols;
                }
                Some("facets") => {
                    let name = need(args, 2, "view facets <on|off>")?;
                    settings.view.show_facets = matches!(name, "on" | "true" | "yes");
                }
                Some("reset") => {
                    settings.view = Default::default();
                }
                Some(other) => {
                    return Err(format!(
                        "unknown view setting '{other}' (layout, theme, density, group-by, columns, facets, reset)"
                    ));
                }
            }
            settings
                .save(&spath)
                .map_err(|e| format!("save settings: {e}"))?;
            let v = &settings.view;
            println!(
                "View: layout={} theme={} density={} group-by={} columns={} facets={}",
                v.layout.name(),
                v.theme.name(),
                v.density.name(),
                v.group_by.name(),
                v.columns
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>()
                    .join(","),
                if v.show_facets { "on" } else { "off" }
            );
            Ok(())
        }
        "facets" => {
            let cat = Catalog::load(&path)?;
            if json_mode {
                let group = |pairs: Vec<(String, usize)>| {
                    Json::Arr(
                        pairs
                            .into_iter()
                            .map(|(name, count)| {
                                Json::Obj(vec![
                                    ("name".into(), Json::Str(name)),
                                    ("count".into(), Json::Num(count as f64)),
                                ])
                            })
                            .collect(),
                    )
                };
                let obj = Json::Obj(vec![
                    ("languages".into(), group(cat.language_facets())),
                    ("topics".into(), group(cat.topic_facets())),
                    ("licenses".into(), group(cat.license_facets())),
                    ("owners".into(), group(cat.owner_facets())),
                ]);
                println!("{}", obj.to_string_pretty());
                return Ok(());
            }
            println!("Languages:");
            for (name, n) in cat.language_facets() {
                println!("  {name} ({n})");
            }
            println!("Topics:");
            for (name, n) in cat.topic_facets() {
                println!("  {name} ({n})");
            }
            println!("Licenses:");
            for (name, n) in cat.license_facets() {
                println!("  {name} ({n})");
            }
            println!("Owners:");
            for (name, n) in cat.owner_facets() {
                println!("  {name} ({n})");
            }
            Ok(())
        }
        "feed-url" => {
            let repo = RepoRef::parse(need(args, 1, "feed-url <owner/repo>")?).map_err(|e| e.0)?;
            println!("{}", repo.releases_atom_url());
            Ok(())
        }
        "check-feed" => {
            // Parse a saved Atom feed file — the offline-testable half of the
            // release watcher. Live fetching joins it in the network slice.
            let file = need(args, 1, "check-feed <releases.atom file>")?;
            let xml = std::fs::read_to_string(file).map_err(|e| format!("read feed file: {e}"))?;
            let notes = parse_releases_atom(&xml);
            if notes.is_empty() {
                println!("No releases in that feed.");
            }
            for n in &notes {
                println!("{}  {}  {}", n.tag, n.updated, n.title);
            }
            Ok(())
        }
        "match" => {
            let pattern = need(args, 1, "match <glob> <asset> [asset...]")?;
            if args.len() < 3 {
                return Err("usage: repostacks match <glob> <asset> [asset...]".into());
            }
            let assets: Vec<String> = args[2..].to_vec();
            let picked = select_assets(&[pattern.to_string()], &assets);
            for a in &picked {
                println!("{a}");
            }
            if picked.is_empty() {
                println!("(no assets match)");
            }
            Ok(())
        }
        "check" => {
            // Live release check over the Atom feed — spends no API quota.
            let repo = RepoRef::parse(need(args, 1, "check <owner/repo>")?).map_err(|e| e.0)?;
            let mut cat = Catalog::load(&path)?;
            if cat.get(&repo).is_none() {
                cat.add(repo.clone());
            }
            let fetcher = CurlFetcher::default();
            let entry = cat.get_mut(&repo).expect("just added");
            let tok = token();
            match check_releases(&fetcher, entry, now_unix(), tok.as_deref()) {
                Ok(out) => {
                    record_usage(UsageKind::CheckFallback, out.quota_spent);
                    let repo = entry.repo.clone();
                    let installed = entry.installed_tag.clone();
                    cat.save(&path)
                        .map_err(|e| format!("save catalogue: {e}"))?;
                    if out.latest_tag.is_empty() {
                        println!("{repo}: no releases in the feed.");
                    } else if !installed.is_empty() && installed != out.latest_tag {
                        println!(
                            "{repo}: latest {} — update available (installed {installed}).",
                            out.latest_tag
                        );
                    } else if out.changed {
                        println!("{repo}: new release {}.", out.latest_tag);
                    } else {
                        println!("{repo}: latest {} (no change).", out.latest_tag);
                    }
                    Ok(())
                }
                Err(msg) => Err(msg),
            }
        }
        "check-all" => {
            let mut cat = Catalog::load(&path)?;
            if cat.entries.is_empty() {
                println!("Your catalogue is empty.");
                return Ok(());
            }
            let settings = Settings::load(&settings_path())?;
            let fetcher = CurlFetcher::default();
            let now = now_unix();
            // Checks run in parallel (bounded by the concurrency setting);
            // results merge back into the catalogue in catalogue order.
            let tok = token();
            let results = check_entries_parallel(
                &fetcher,
                &cat.entries,
                now,
                settings.concurrency as usize,
                tok.as_deref(),
            );
            let mut failures = 0;
            for r in &results {
                if let Some(entry) = cat.get_mut(&r.repo) {
                    entry.latest_tag = r.entry.latest_tag.clone();
                    entry.last_release_check = r.entry.last_release_check;
                }
                match &r.outcome {
                    Ok(out) => {
                        if out.changed {
                            println!("{}: new release {}", r.repo, out.latest_tag);
                        } else if !out.latest_tag.is_empty() {
                            println!("{}: {} (no change)", r.repo, out.latest_tag);
                        } else {
                            println!("{}: no releases", r.repo);
                        }
                    }
                    Err(msg) => {
                        failures += 1;
                        println!("{}: {msg}", r.repo);
                    }
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            let fallback_spent: u32 = results
                .iter()
                .filter_map(|r| r.outcome.as_ref().ok())
                .map(|o| o.quota_spent)
                .sum();
            record_usage(UsageKind::CheckFallback, fallback_spent);
            if fallback_spent == 0 {
                println!("Release checks used no API quota (Atom feeds).");
            } else {
                println!(
                    "Release checks rode Atom feeds; {fallback_spent} repo(s) had no feed and used the API fallback (1 call each)."
                );
            }
            if failures > 0 {
                Err(format!("{failures} check(s) failed"))
            } else {
                Ok(())
            }
        }
        "refresh" => {
            // Metadata refresh under the cache policy. Usage:
            // refresh <owner/repo> [--force] | refresh --all [--force]
            let force = args.iter().any(|a| a == "--force");
            let all = args.iter().any(|a| a == "--all");
            let tok = token();
            let policy = CachePolicy::for_auth(auth_for_token(tok.as_deref()));
            let fetcher = CurlFetcher::default();
            let mut cat = Catalog::load(&path)?;
            let targets: Vec<RepoRef> = if all {
                cat.entries.iter().map(|e| e.repo.clone()).collect()
            } else {
                let first = args.get(1).map(String::as_str).unwrap_or("");
                vec![RepoRef::parse(first).map_err(|e| e.0)?]
            };
            if targets.is_empty() {
                println!("Your catalogue is empty.");
                return Ok(());
            }
            // Refresh in parallel (bounded by the concurrency setting):
            // snapshots go out, refreshed copies merge back — the same
            // shape as check --all.
            for repo in &targets {
                if cat.get(repo).is_none() {
                    println!("{repo}: not in the catalogue (add it first).");
                }
            }
            let settings = Settings::load(&settings_path())?;
            let snapshots: Vec<RepoEntry> =
                targets.iter().filter_map(|r| cat.get(r).cloned()).collect();
            let results = refresh_entries_parallel(
                &fetcher,
                &snapshots,
                &policy,
                tok.as_deref(),
                now_unix(),
                force,
                settings.concurrency as usize,
            );
            let mut spent = 0u32;
            for r in results {
                let repo = &r.repo;
                match r.outcome {
                    Ok(
                        outcome @ (RefreshOutcome::Updated { .. }
                        | RefreshOutcome::NotModified { .. }),
                    ) => {
                        if let Some(slot) = cat.get_mut(repo) {
                            *slot = r.entry;
                        }
                        match outcome {
                            RefreshOutcome::Updated { quota_spent } => {
                                spent += quota_spent;
                                println!("{repo}: metadata updated.");
                            }
                            RefreshOutcome::NotModified { quota_spent } => {
                                spent += quota_spent;
                                println!("{repo}: unchanged.");
                            }
                            RefreshOutcome::SkippedNotDue => unreachable!(),
                        }
                    }
                    Ok(RefreshOutcome::SkippedNotDue) => {
                        println!("{repo}: cached metadata is still fresh (policy) — use --force to override.");
                    }
                    Err(msg) => println!("{repo}: {msg}"),
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            record_usage(UsageKind::Metadata, spent);
            println!(
                "API quota spent this run: {spent} of {} per hour.",
                policy.auth.hourly_rest_limit()
            );
            Ok(())
        }
        "assets" => {
            // The asset list of a release: one REST call, reported honestly.
            let repo =
                RepoRef::parse(need(args, 1, "assets <owner/repo> [tag]")?).map_err(|e| e.0)?;
            let cat = Catalog::load(&path)?;
            let entry = cat
                .get(&repo)
                .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
            let tag = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| entry.latest_tag.clone());
            if tag.is_empty() {
                return Err(format!(
                    "no tag known for {repo} — run `check {repo}` first or pass a tag"
                ));
            }
            let fetcher = CurlFetcher::default();
            let tok = token();
            let (assets, spent) = fetch_release_assets(&fetcher, &repo, &tag, tok.as_deref())?;
            record_usage(UsageKind::Assets, spent);
            println!("{repo} {tag}: {} asset(s)", assets.len());
            for a in &assets {
                println!(
                    "  {}  ({} bytes){}",
                    a.name,
                    a.size,
                    if a.digest.is_empty() {
                        ""
                    } else {
                        "  [sha256 published]"
                    }
                );
            }
            match plan_download(
                entry,
                &tag,
                &assets,
                std::path::Path::new(&Settings::load(&settings_path())?.download_dir),
            ) {
                Ok(plan) => println!("Auto-pick: {} — {}", plan.asset_name, plan.reason),
                Err(msg) => println!("Auto-pick: none ({msg})"),
            }
            let suggested = suggest_asset_patterns(&assets, &tag);
            if !suggested.is_empty() {
                println!(
                    "Suggested rules (repostacks rules {repo} {}):",
                    suggested.join(" ")
                );
                for p in &suggested {
                    println!("  {p}");
                }
            }
            println!("API quota spent: {spent}.");
            Ok(())
        }
        "rules" => {
            // Per-volume asset rules: show with no patterns, set with them.
            let repo =
                RepoRef::parse(need(args, 1, "rules <owner/repo> [glob...]")?).map_err(|e| e.0)?;
            let mut cat = Catalog::load(&path)?;
            if args.len() > 2 {
                let patterns: Vec<String> = args[2..].to_vec();
                let entry = cat
                    .get_mut(&repo)
                    .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
                entry.asset_patterns = patterns;
                cat.save(&path)
                    .map_err(|e| format!("save catalogue: {e}"))?;
                println!("{repo}: asset rules set.");
            }
            let entry = cat
                .get(&repo)
                .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
            if entry.asset_patterns.is_empty() {
                println!("{repo}: no rules — the Windows-first default pick applies.");
            } else {
                println!("{repo} rules:");
                for pat in &entry.asset_patterns {
                    println!("  {pat}");
                }
            }
            Ok(())
        }
        "auto" => {
            // Standing order: auto-download new releases when the Librarian runs.
            let repo =
                RepoRef::parse(need(args, 1, "auto <owner/repo> <on|off>")?).map_err(|e| e.0)?;
            let on = match need(args, 2, "auto <owner/repo> <on|off>")? {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                other => return Err(format!("expected on or off, got '{other}'")),
            };
            let mut cat = Catalog::load(&path)?;
            let entry = cat
                .get_mut(&repo)
                .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
            entry.auto_download = on;
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            println!("{repo}: standing order {}.", if on { "ON" } else { "OFF" });
            Ok(())
        }
        "download" => {
            let repo =
                RepoRef::parse(need(args, 1, "download <owner/repo> [tag]")?).map_err(|e| e.0)?;
            let settings = Settings::load(&settings_path())?;
            let root = PathBuf::from(&settings.download_dir);
            let mut cat = Catalog::load(&path)?;
            let tag = match args.get(2) {
                Some(t) => t.clone(),
                None => {
                    let entry = cat
                        .get(&repo)
                        .ok_or_else(|| format!("{repo} is not in the catalogue"))?;
                    if entry.latest_tag.is_empty() {
                        return Err(format!(
                            "no tag known for {repo} — run `check {repo}` first"
                        ));
                    }
                    entry.latest_tag.clone()
                }
            };
            if cat.get(&repo).is_none() {
                return Err(format!("{repo} is not in the catalogue"));
            }
            let fetcher = CurlFetcher::default();
            let tok = token();
            let (assets, spent) = fetch_release_assets(&fetcher, &repo, &tag, tok.as_deref())?;
            record_usage(UsageKind::Assets, spent);
            let entry = cat.get(&repo).expect("checked").clone();
            let mut queue = Queue::load(&queue_path())?;
            let id = enqueue_download(&mut queue, &entry, &tag, &assets, &root)?;
            println!(
                "Queued job {id}: {repo} {tag} (assets fetched, quota {spent}).",
                repo = entry.repo
            );
            let downloader = CurlDownloader::default();
            let outcomes = run_pending(
                &mut queue,
                &mut cat,
                &downloader,
                &root,
                tok.as_deref(),
                settings.concurrency as usize,
            );
            queue
                .save(&queue_path())
                .map_err(|e| format!("save queue: {e}"))?;
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            match outcomes.first() {
                Some(o) if o.status == JobStatus::Done => {
                    println!("Downloaded to {}", o.final_path.display());
                    Ok(())
                }
                Some(o) => Err(format!("download failed: {}", o.error)),
                None => Err("nothing ran".into()),
            }
        }
        "update" => {
            // Check (Atom, free) then download the newer release (1 call).
            let repo = RepoRef::parse(need(args, 1, "update <owner/repo>")?).map_err(|e| e.0)?;
            let mut cat = Catalog::load(&path)?;
            if cat.get(&repo).is_none() {
                return Err(format!("{repo} is not in the catalogue"));
            }
            let fetcher = CurlFetcher::default();
            let now = now_unix();
            let tok = token();
            let (latest, installed) = {
                let entry = cat.get_mut(&repo).expect("checked");
                let out = check_releases(&fetcher, entry, now, tok.as_deref())?;
                record_usage(UsageKind::CheckFallback, out.quota_spent);
                (out.latest_tag, entry.installed_tag.clone())
            };
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            if latest.is_empty() {
                println!("{repo}: no releases.");
                return Ok(());
            }
            if latest == installed {
                println!("{repo}: {latest} already installed.");
                return Ok(());
            }
            println!(
                "{repo}: {latest} available (installed: {}). Downloading…",
                if installed.is_empty() {
                    "none".into()
                } else {
                    installed
                }
            );
            // Reuse the download path with an explicit tag.
            let sub: Vec<String> = vec!["download".into(), repo.full_name(), latest];
            run(&sub)
        }
        "update-all" => {
            let mut cat = Catalog::load(&path)?;
            let fetcher = CurlFetcher::default();
            let now = now_unix();
            let repos: Vec<RepoRef> = cat.entries.iter().map(|e| e.repo.clone()).collect();
            let mut todo: Vec<RepoRef> = Vec::new();
            let tok = token();
            for repo in repos {
                let entry = cat.get_mut(&repo).expect("listed");
                match check_releases(&fetcher, entry, now, tok.as_deref()) {
                    Ok(out) => {
                        record_usage(UsageKind::CheckFallback, out.quota_spent);
                        if !out.latest_tag.is_empty() && out.latest_tag != entry.installed_tag {
                            println!("{repo}: {} available.", out.latest_tag);
                            todo.push(repo.clone());
                        } else {
                            println!("{repo}: up to date.");
                        }
                    }
                    Err(msg) => println!("{repo}: {msg}"),
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            let mut failed = 0;
            for repo in todo {
                let sub: Vec<String> = vec!["download".into(), repo.full_name()];
                if run(&sub).is_err() {
                    failed += 1;
                }
            }
            if failed > 0 {
                Err(format!("{failed} update(s) failed"))
            } else {
                Ok(())
            }
        }
        "queue" => {
            let qpath = queue_path();
            let mut queue = Queue::load(&qpath)?;
            match args.get(1).map(String::as_str) {
                None | Some("list") => {
                    if json_mode {
                        let arr = Json::Arr(
                            queue
                                .jobs
                                .iter()
                                .map(|j| {
                                    Json::Obj(vec![
                                        ("id".into(), Json::Num(j.id as f64)),
                                        ("repo".into(), Json::Str(j.repo.full_name())),
                                        ("tag".into(), Json::Str(j.tag.clone())),
                                        ("asset_name".into(), Json::Str(j.asset_name.clone())),
                                        (
                                            "status".into(),
                                            Json::Str(j.status.label().to_string()),
                                        ),
                                        ("bytes_done".into(), Json::Num(j.bytes_done as f64)),
                                        ("bytes_total".into(), Json::Num(j.bytes_total as f64)),
                                        (
                                            "percent".into(),
                                            Json::Num(j.progress_percent()),
                                        ),
                                        ("error".into(), Json::Str(j.error.clone())),
                                    ])
                                })
                                .collect(),
                        );
                        println!("{}", arr.to_string_pretty());
                        return Ok(());
                    }
                    if queue.jobs.is_empty() {
                        println!("The acquisitions queue is empty.");
                    }
                    for j in &queue.jobs {
                        println!(
                            "#{id} {repo} {tag} · {asset} · {status} · {done}/{total} bytes{err}",
                            id = j.id,
                            repo = j.repo,
                            tag = j.tag,
                            asset = j.asset_name,
                            status = j.status.label(),
                            done = j.bytes_done,
                            total = j.bytes_total,
                            err = if j.error.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", j.error)
                            }
                        );
                    }
                    Ok(())
                }
                Some("run") => {
                    let settings = Settings::load(&settings_path())?;
                    let root = PathBuf::from(&settings.download_dir);
                    let mut cat = Catalog::load(&path)?;
                    let downloader = CurlDownloader::default();
                    let tok = token();
                    let outcomes = run_pending(
                        &mut queue,
                        &mut cat,
                        &downloader,
                        &root,
                        tok.as_deref(),
                        settings.concurrency as usize,
                    );
                    queue.save(&qpath).map_err(|e| format!("save queue: {e}"))?;
                    cat.save(&path)
                        .map_err(|e| format!("save catalogue: {e}"))?;
                    let failed = outcomes
                        .iter()
                        .filter(|o| o.status == JobStatus::Failed)
                        .count();
                    println!("Ran {} job(s); {} failed.", outcomes.len(), failed);
                    if failed > 0 {
                        Err("some downloads failed — see queue list".into())
                    } else {
                        Ok(())
                    }
                }
                Some("pause") | Some("resume") | Some("cancel") => {
                    let verb = args[1].clone();
                    let id: u64 = need(args, 2, "queue <pause|resume|cancel> <id>")?
                        .parse()
                        .map_err(|_| "job id must be a number".to_string())?;
                    let ok = match verb.as_str() {
                        "pause" => queue.pause(id),
                        "resume" => queue.resume(id),
                        _ => queue.cancel(id),
                    };
                    if ok {
                        queue.save(&qpath).map_err(|e| format!("save queue: {e}"))?;
                        println!("Job {id}: {verb} applied.");
                        Ok(())
                    } else {
                        Err(format!("job {id} cannot {verb} from its current state"))
                    }
                }
                Some("clear") => {
                    let n = queue.clear_finished();
                    queue.save(&qpath).map_err(|e| format!("save queue: {e}"))?;
                    println!("Cleared {n} finished job(s).");
                    Ok(())
                }
                Some("retry") => {
                    // retry <id> retries one failed job; retry all retries
                    // every failed job. A kept .part file makes it resume.
                    match args.get(2).map(String::as_str) {
                        Some("all") => {
                            let ids: Vec<u64> = queue
                                .jobs
                                .iter()
                                .filter(|j| j.status == JobStatus::Failed)
                                .map(|j| j.id)
                                .collect();
                            let n = ids.iter().filter(|id| queue.retry(**id)).count();
                            queue.save(&qpath).map_err(|e| format!("save queue: {e}"))?;
                            println!("Requeued {n} failed job(s). Run `queue run` to start them.");
                            Ok(())
                        }
                        Some(id_text) => {
                            let id: u64 = id_text
                                .parse()
                                .map_err(|_| "job id must be a number (or 'all')".to_string())?;
                            if queue.retry(id) {
                                queue
                                    .save(&qpath)
                                    .map_err(|e| format!("save queue: {e}"))?;
                                println!("Job {id}: requeued. Run `queue run` to start it.");
                                Ok(())
                            } else {
                                Err(format!("job {id} is not failed — only failed jobs retry"))
                            }
                        }
                        None => Err("usage: repostacks queue retry <id|all>".into()),
                    }
                }
                Some(other) => Err(format!(
                    "unknown queue action '{other}' (list, run, pause, resume, cancel, retry, clear)"
                )),
            }
        }
        "config" => {
            let spath = settings_path();
            let mut settings = Settings::load(&spath)?;
            match args.get(1).map(String::as_str) {
                None => {
                    println!("download-dir: {}", settings.download_dir);
                    println!("concurrency:  {}", settings.concurrency);
                    println!("atom-poll:    {}s", settings.atom_poll_secs);
                    println!("metadata-poll: {}s", settings.metadata_poll_secs);
                    Ok(())
                }
                Some("download-dir") => {
                    settings.download_dir =
                        need(args, 2, "config download-dir <path>")?.to_string();
                    settings.validate()?;
                    settings
                        .save(&spath)
                        .map_err(|e| format!("save settings: {e}"))?;
                    println!("download-dir: {}", settings.download_dir);
                    Ok(())
                }
                Some("concurrency") => {
                    let n: u32 = need(args, 2, "config concurrency <n>")?
                        .parse()
                        .map_err(|_| "concurrency must be a number".to_string())?;
                    settings.concurrency = n;
                    settings.validate()?;
                    settings
                        .save(&spath)
                        .map_err(|e| format!("save settings: {e}"))?;
                    println!("concurrency: {}", settings.concurrency);
                    Ok(())
                }
                Some("poll") => {
                    let atom: u64 = need(args, 2, "config poll <atom-secs> <metadata-secs>")?
                        .parse()
                        .map_err(|_| "poll seconds must be a number".to_string())?;
                    let meta: u64 = need(args, 3, "config poll <atom-secs> <metadata-secs>")?
                        .parse()
                        .map_err(|_| "poll seconds must be a number".to_string())?;
                    settings.atom_poll_secs = atom;
                    settings.metadata_poll_secs = meta;
                    settings.validate()?;
                    settings
                        .save(&spath)
                        .map_err(|e| format!("save settings: {e}"))?;
                    println!("atom-poll: {atom}s · metadata-poll: {meta}s");
                    Ok(())
                }
                Some(other) => Err(format!(
                    "unknown config key '{other}' (download-dir, concurrency, poll)"
                )),
            }
        }
        "updates" => {
            let cat = Catalog::load(&path)?;
            let ups = cat.updates_available();
            if json_mode {
                print_entries_json(&ups, &cat);
                return Ok(());
            }
            if ups.is_empty() {
                println!("Everything installed is up to date.");
            }
            for e in ups {
                println!("{}: {} -> {}", e.repo, e.installed_tag, e.latest_tag);
            }
            Ok(())
        }
        "usage" => {
            // The standing API meter: calls RepoStacks itself counted,
            // grouped by purpose. This is not a live GitHub reading —
            // other tools using the same address or token are not in it.
            let ledger = UsageLedger::load(&usage_path())?;
            let tok = token();
            let snap = ledger.snapshot(auth_for_token(tok.as_deref()), now_unix());
            if json_mode {
                let kinds = |hour: bool, day: bool, life: bool| {
                    Json::Obj(
                        UsageKind::all()
                            .into_iter()
                            .map(|kind| {
                                let value = if hour {
                                    snap.hour_by_kind.get(&kind).copied().unwrap_or(0) as f64
                                } else if day {
                                    snap.day_by_kind.get(&kind).copied().unwrap_or(0) as f64
                                } else if life {
                                    snap.lifetime_by_kind.get(&kind).copied().unwrap_or(0) as f64
                                } else {
                                    0.0
                                };
                                (kind.name().into(), Json::Num(value))
                            })
                            .collect(),
                    )
                };
                let out = Json::Obj(vec![
                    (
                        "mode".into(),
                        Json::Str(if tok.is_some() { "token" } else { "anonymous" }.into()),
                    ),
                    ("hourly_limit".into(), Json::Num(snap.hourly_limit as f64)),
                    ("hour_spent".into(), Json::Num(snap.hour_spent as f64)),
                    ("hour_percent".into(), Json::Num(snap.hour_percent())),
                    (
                        "hour_reset_unix".into(),
                        Json::Num(snap.hour_reset_unix as f64),
                    ),
                    ("day_spent".into(), Json::Num(snap.day_spent as f64)),
                    (
                        "lifetime_spent".into(),
                        Json::Num(snap.lifetime_spent as f64),
                    ),
                    (
                        "last_spent_unix".into(),
                        Json::Num(snap.last_spent_unix as f64),
                    ),
                    ("hour_by_kind".into(), kinds(true, false, false)),
                    ("day_by_kind".into(), kinds(false, true, false)),
                    ("lifetime_by_kind".into(), kinds(false, false, true)),
                ]);
                println!("{}", out.to_string_pretty());
                return Ok(());
            }
            let reset_secs = snap.hour_reset_unix % 86_400;
            println!("API usage counted by RepoStacks (not a live GitHub reading):");
            println!(
                "  This hour: {} of {} call(s) ({:.0}%), window resets {:02}:{:02} UTC.",
                snap.hour_spent,
                snap.hourly_limit,
                snap.hour_percent(),
                reset_secs / 3600,
                (reset_secs % 3600) / 60
            );
            println!(
                "  Today (UTC): {} call(s). Lifetime: {} call(s).",
                snap.day_spent, snap.lifetime_spent
            );
            for kind in UsageKind::all() {
                let lifetime = snap.lifetime_by_kind.get(&kind).copied().unwrap_or(0);
                if lifetime > 0 {
                    println!(
                        "  {}: {} this hour, {} today, {} lifetime.",
                        kind.label(),
                        snap.hour_by_kind.get(&kind).copied().unwrap_or(0),
                        snap.day_by_kind.get(&kind).copied().unwrap_or(0),
                        lifetime
                    );
                }
            }
            if snap.lifetime_spent == 0 {
                println!("  No counted REST calls yet. Atom release checks are free.");
            }
            Ok(())
        }
        "bench" => {
            // Performance evidence: time the core operations over a
            // synthetic catalogue. No network, and no contact with the
            // real catalogue — the scratch catalogue lives in a temp
            // folder and is removed afterwards.
            let mut repos = 5000usize;
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--repos" {
                    if let Some(v) = args.get(i + 1) {
                        repos = v
                            .parse()
                            .map_err(|_| "--repos needs a number".to_string())?;
                    }
                }
                i += 1;
            }
            let dir = std::env::temp_dir().join(format!("repostacks-bench-{}", std::process::id()));
            let report = repostacks_core::bench::run_bench(repos, &dir)?;
            let _ = std::fs::remove_dir_all(&dir);
            if json_mode {
                let scenarios = Json::Arr(
                    report
                        .scenarios
                        .iter()
                        .map(|s| {
                            Json::Obj(vec![
                                ("name".into(), Json::Str(s.name.clone())),
                                ("detail".into(), Json::Str(s.detail.clone())),
                                ("iterations".into(), Json::Num(s.iterations as f64)),
                                ("median_ms".into(), Json::Num(s.median_ms)),
                                ("min_ms".into(), Json::Num(s.min_ms)),
                            ])
                        })
                        .collect(),
                );
                println!(
                    "{}",
                    Json::Obj(vec![
                        ("repos".into(), Json::Num(report.repos as f64)),
                        (
                            "catalog_bytes".into(),
                            Json::Num(report.catalog_bytes as f64)
                        ),
                        ("scenarios".into(), scenarios),
                    ])
                    .to_string_pretty()
                );
            } else {
                println!(
                    "RepoStacks bench - synthetic catalogue of {} volumes ({} KB on disk)",
                    report.repos,
                    report.catalog_bytes / 1024
                );
                for s in &report.scenarios {
                    println!(
                        "  {:<18} {:<52} median {:>9.2} ms   min {:>9.2} ms",
                        s.name, s.detail, s.median_ms, s.min_ms
                    );
                }
            }
            Ok(())
        }
        "doctor" => {
            // Plain-language health check: state files readable, data
            // folder writable, curl present. Exit code fails if any check
            // fails, so scripts can gate on it too.
            let mut bad = 0usize;
            let mut report = |ok: bool, label: String| {
                println!("{} {label}", if ok { "OK   " } else { "FAIL " });
                if !ok {
                    bad += 1;
                }
            };
            match Catalog::load(&path) {
                Ok(c) => report(
                    true,
                    format!("catalogue loads ({} volume(s))", c.entries.len()),
                ),
                Err(e) => report(false, format!("catalogue: {e}")),
            }
            match Queue::load(&queue_path()) {
                Ok(q) => report(true, format!("queue loads ({} job(s))", q.jobs.len())),
                Err(e) => report(false, format!("queue: {e}")),
            }
            match Settings::load(&settings_path()) {
                Ok(_) => report(true, "settings load".into()),
                Err(e) => report(false, format!("settings: {e}")),
            }
            match UsageLedger::load(&usage_path()) {
                Ok(u) => report(
                    true,
                    format!(
                        "usage ledger loads ({} lifetime counted call(s))",
                        u.snapshot(auth_for_token(token().as_deref()), now_unix())
                            .lifetime_spent
                    ),
                ),
                Err(e) => report(false, format!("usage ledger: {e}")),
            }
            let dir = path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let probe = dir.join(".repostacks-doctor-probe");
            let writable = std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&probe, b"ok"))
                .and_then(|_| std::fs::remove_file(&probe))
                .is_ok();
            report(
                writable,
                format!("data folder {} is writable", dir.display()),
            );
            let curl_ok = hidden_command("curl")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            report(
                curl_ok,
                "curl is available (network checks and downloads use it)".into(),
            );
            // Library health (informational, never a failure): what's
            // stale, what's stuck, and what a `clean` would reclaim.
            if let (Ok(c), Ok(q)) = (Catalog::load(&path), Queue::load(&queue_path())) {
                let now = now_unix();
                let never = c
                    .entries
                    .iter()
                    .filter(|e| e.last_release_check == 0)
                    .count();
                let due = c
                    .entries
                    .iter()
                    .filter(|e| {
                        e.last_release_check == 0
                            || now.saturating_sub(e.last_release_check)
                                > repostacks_core::search::DUE_CHECK_SECS
                    })
                    .count();
                let failed = q
                    .jobs
                    .iter()
                    .filter(|j| j.status == JobStatus::Failed)
                    .count();
                println!(
                    "INFO  library health: {} never checked, {} due for a check, {} failed job(s)",
                    never, due, failed
                );
                if let Ok(s) = Settings::load(&settings_path()) {
                    let scan = scan_downloads(&PathBuf::from(&s.download_dir), &c, &q);
                    let items = scan.stale_folders.len() + scan.orphan_parts.len();
                    if items > 0 {
                        println!(
                            "INFO  clean would reclaim {} item(s), {:.1} MB (run `repostacks clean`)",
                            items,
                            scan.bytes as f64 / 1_000_000.0
                        );
                    }
                }
            }
            if bad > 0 {
                Err(format!("{bad} check(s) failed"))
            } else {
                println!("All checks passed.");
                Ok(())
            }
        }
        other => Err(format!("unknown command '{other}' — run `repostacks help`")),
    }
}

fn need<'a>(args: &'a [String], idx: usize, usage: &str) -> Result<&'a str, String> {
    args.get(idx)
        .map(String::as_str)
        .ok_or_else(|| format!("usage: repostacks {usage}"))
}

fn print_help() {
    println!(
        "RepoStacks — your GitHub library\n\n\
         Usage: repostacks <command>\n\n\
         Commands:\n  \
         add <owner/repo|URL>     Add a volume to the catalogue\n  \
         import <file|-> [shelf]  Bulk import, one repo per line, with preview counts\n  \
         import-user <name> [filters]  Import a GitHub user's projects (--owned/--member, --forks/--sources, --with-archived/--only-archived, --public/--private, --dry-run)\n  \
         import-org <name> [filters]  Import a GitHub organization's projects (same filters, minus --owned/--member)\n  \
         export [--json] [file]    Export the catalogue (importable list, or full JSON)\n  \
         clean [--yes]             Remove stale download folders + orphan .part files (dry run without --yes)\n  \
         remove <owner/repo>       Remove a volume (and its shelf places)\n  \
         list                      List the catalogue with install/update badges\n  \
         shelf add <name> <repo>   Put a volume on a shelf\n  \
         shelf list <name>         Show a shelf\n  \
         shelf update <name>       Check a shelf's members and download their updates\n  \
         search <text>             Search name, description, language, topics\n  \
         facets                    Language and topic counts for the filter panel\n  \
         find <query>              Rich search: lang: topic: license: owner: shelf:NAME tag:v1 stars:>1000 has:update is:installed is:due sort:-stars limit:10; prefix any term with - to exclude it\n  \
         search save|list|run|delete  Saved searches\n  \
         view [layout|theme|density|group-by|columns|facets|reset]  Presentation settings\n  \
         feed-url <owner/repo>     Print the releases Atom feed URL (quota-free checks)\n  \
         check-feed <file.atom>    Read a saved Atom feed and list its releases\n  \
         check <owner/repo>        Live release check over Atom (1 API call only when a repo has no feed)\n  \
         check-all                 Live release check for the whole catalogue\n  \
         refresh <repo|--all> [--force]  Refresh metadata under the cache policy\n  \
         match <glob> <asset>...   Test an asset-selection rule\n  \
         assets <repo> [tag]       List a release's downloadable assets (1 API call)\n  \
         rules <repo> [glob...]    Show or set a volume's asset-selection rules\n  \
         auto <repo> <on|off>      Standing order: auto-download new releases\n  \
         download <repo> [tag]     Download a release now (queue + engine)\n  \
         update <repo>             Check (free) and download if newer\n  \
         update-all                Check everything and download what's newer\n  \
         queue [list|run|pause|resume|cancel|retry|clear]  The acquisitions queue\n  \
         config [download-dir|concurrency|poll]  Librarian settings\n  \
         updates                   Installed volumes with a newer release\n  \
         usage                     API calls RepoStacks has counted, by purpose and period\n  \
         bench [--repos N]         Time the core operations over a synthetic catalogue (default 5000 volumes)\n  \
         doctor                    Health check: state files, data folder, curl\n  \
         --json                    With list/find/updates/facets/queue list: JSON output\n  \
         version                   Print the version\n"
    );
}
