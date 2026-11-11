//! RepoStacks command line — every window capability is a command first.
//!
//! Data file: `REPOSTACKS_DATA` env var, else `data/catalog.json` beside the
//! working directory. The file carries `schema_version` and is written
//! atomically, so a crash never loses the catalogue.

use repostacks_core::atom::parse_releases_atom;
use repostacks_core::catalog::{Catalog, Filter};
use repostacks_core::glob_match::select_assets;
use repostacks_core::RepoRef;
use std::path::PathBuf;
use std::process::ExitCode;

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
        "search" => {
            let text = need(args, 1, "search <text>")?;
            let cat = Catalog::load(&path)?;
            let hits = cat.filter(&Filter {
                text: text.to_string(),
                ..Default::default()
            });
            if hits.is_empty() {
                println!("No volumes match '{text}'.");
            }
            for e in hits {
                println!("{}", e.repo);
            }
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
         remove <owner/repo>       Remove a volume (and its shelf places)\n  \
         list                      List the catalogue with install/update badges\n  \
         shelf add <name> <repo>   Put a volume on a shelf\n  \
         shelf list <name>         Show a shelf\n  \
         search <text>             Search name, description, language, topics\n  \
         facets                    Language and topic counts for the filter panel\n  \
         feed-url <owner/repo>     Print the releases Atom feed URL (quota-free checks)\n  \
         check-feed <file.atom>    Read a saved Atom feed and list its releases\n  \
         match <glob> <asset>...   Test an asset-selection rule\n  \
         updates                   Installed volumes with a newer release\n  \
         version                   Print the version\n"
    );
}
