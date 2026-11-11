//! RepoStacks command line — every window capability is a command first.
//!
//! Data file: `REPOSTACKS_DATA` env var, else `data/catalog.json` beside the
//! working directory. The file carries `schema_version` and is written
//! atomically, so a crash never loses the catalogue.

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::cache_policy::CachePolicy;
use repostacks_core::catalog::Catalog;
use repostacks_core::glob_match::select_assets;
use repostacks_core::http::CurlFetcher;
use repostacks_core::net::{auth_for_token, check_releases, refresh_metadata, RefreshOutcome};
use repostacks_core::search::{execute as search_execute, Query};
use repostacks_core::view::{render, Column, Density, GroupBy, Layout, Settings, Theme};
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

fn settings_path() -> PathBuf {
    match std::env::var_os("REPOSTACKS_SETTINGS") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("data").join("settings.json"),
    }
}

fn run_find(catalog: &Catalog, settings: &Settings, query_text: &str) -> Result<(), String> {
    let q = Query::parse(query_text);
    let hits = search_execute(catalog, &q);
    print!("{}", render(&hits, catalog, &settings.view));
    println!("{} volume(s).", hits.len());
    Ok(())
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
                _ => Err(
                    "usage: repostacks shelf add <name> <owner/repo> | shelf list <name>".into(),
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
            println!("Languages:");
            for (name, n) in cat.language_facets() {
                println!("  {name} ({n})");
            }
            println!("Topics:");
            for (name, n) in cat.topic_facets() {
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
            match check_releases(&fetcher, entry, now_unix()) {
                Ok(out) => {
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
            let fetcher = CurlFetcher::default();
            let now = now_unix();
            let mut failures = 0;
            let repos: Vec<RepoRef> = cat.entries.iter().map(|e| e.repo.clone()).collect();
            for repo in repos {
                let entry = cat.get_mut(&repo).expect("listed");
                match check_releases(&fetcher, entry, now) {
                    Ok(out) => {
                        if out.changed {
                            println!("{repo}: new release {}", out.latest_tag);
                        } else if !out.latest_tag.is_empty() {
                            println!("{repo}: {} (no change)", out.latest_tag);
                        } else {
                            println!("{repo}: no releases");
                        }
                    }
                    Err(msg) => {
                        failures += 1;
                        println!("{repo}: {msg}");
                    }
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            println!("Release checks used no API quota (Atom feeds).");
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
            let mut spent = 0u32;
            for repo in targets {
                if cat.get(&repo).is_none() {
                    println!("{repo}: not in the catalogue (add it first).");
                    continue;
                }
                let entry = cat.get_mut(&repo).expect("checked");
                match refresh_metadata(&fetcher, entry, &policy, tok.as_deref(), now_unix(), force)
                {
                    Ok(RefreshOutcome::Updated { quota_spent }) => {
                        spent += quota_spent;
                        println!("{repo}: metadata updated.");
                    }
                    Ok(RefreshOutcome::NotModified { quota_spent }) => {
                        spent += quota_spent;
                        println!("{repo}: unchanged.");
                    }
                    Ok(RefreshOutcome::SkippedNotDue) => {
                        println!("{repo}: cached metadata is still fresh (policy) — use --force to override.");
                    }
                    Err(msg) => println!("{repo}: {msg}"),
                }
            }
            cat.save(&path)
                .map_err(|e| format!("save catalogue: {e}"))?;
            println!(
                "API quota spent this run: {spent} of {} per hour.",
                policy.auth.hourly_rest_limit()
            );
            Ok(())
        }
        "updates" => {
            let cat = Catalog::load(&path)?;
            let ups = cat.updates_available();
            if ups.is_empty() {
                println!("Everything installed is up to date.");
            }
            for e in ups {
                println!("{}: {} -> {}", e.repo, e.installed_tag, e.latest_tag);
            }
            Ok(())
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
         remove <owner/repo>       Remove a volume (and its shelf places)\n  \
         list                      List the catalogue with install/update badges\n  \
         shelf add <name> <repo>   Put a volume on a shelf\n  \
         shelf list <name>         Show a shelf\n  \
         search <text>             Search name, description, language, topics\n  \
         facets                    Language and topic counts for the filter panel\n  \
         find <query>              Rich search: lang: topic: license: owner: shelf:NAME stars:>1000 has:update is:installed sort:-stars limit:10\n  \
         search save|list|run|delete  Saved searches\n  \
         view [layout|theme|density|group-by|columns|facets|reset]  Presentation settings\n  \
         feed-url <owner/repo>     Print the releases Atom feed URL (quota-free checks)\n  \
         check-feed <file.atom>    Read a saved Atom feed and list its releases\n  \
         check <owner/repo>        Live release check over Atom (no API quota)\n  \
         check-all                 Live release check for the whole catalogue\n  \
         refresh <repo|--all> [--force]  Refresh metadata under the cache policy\n  \
         match <glob> <asset>...   Test an asset-selection rule\n  \
         updates                   Installed volumes with a newer release\n  \
         version                   Print the version\n"
    );
}
