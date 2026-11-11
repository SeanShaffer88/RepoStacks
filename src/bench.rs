//! Performance measurements over the core operations, run against a
//! synthetic catalogue of a stated size. This is the evidence behind the
//! Performance row in QUALITY.md: the scenarios time the same code paths
//! the app itself uses (catalogue save/load through the persistence
//! layer, the rich search, the facet counts, the Atom parser, and the
//! parallel release check with the network replaced by a stub fetcher),
//! so the numbers describe RepoStacks' own work, never the network.
//!
//! Everything here is deterministic: the synthetic catalogue and feed
//! come from a fixed-seed generator, so runs are comparable over time.

use crate::atom::parse_releases_atom;
use crate::catalog::{Catalog, RepoEntry};
use crate::http::{FetchResponse, Fetcher};
use crate::net::check_entries_parallel;
use crate::repo_ref::RepoRef;
use crate::search::{execute as search_execute, Query};
use std::path::Path;
use std::time::Instant;

/// One measured scenario: how often it ran and the median / best time.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchScenario {
    pub name: String,
    pub detail: String,
    pub iterations: usize,
    pub median_ms: f64,
    pub min_ms: f64,
}

/// The full bench run: the catalogue size it ran against, the on-disk
/// size of that catalogue, and one row per scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchReport {
    pub repos: usize,
    pub catalog_bytes: u64,
    pub scenarios: Vec<BenchScenario>,
}

/// Fixed-seed linear congruential generator — the synthetic data must
/// not change between runs, or the timings stop being comparable.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

const NAME_WORDS: [&str; 10] = [
    "ledger", "stack", "forge", "weave", "harbor", "lens", "vault", "atlas", "prism", "tide",
];
const LANGUAGES: [&str; 8] = [
    "Rust",
    "Go",
    "C++",
    "Python",
    "TypeScript",
    "C#",
    "Java",
    "Zig",
];
const LICENSES: [&str; 5] = ["MIT", "Apache-2.0", "GPL-3.0", "BSD-3-Clause", ""];
const TOPICS: [&str; 10] = [
    "cli", "gui", "library", "network", "database", "editor", "game", "devtools", "security",
    "media",
];

/// Build a catalogue of `n` volumes with realistic spread: a few
/// hundred owners, mixed languages / licenses / topics, most volumes
/// carrying a known latest release and some installed. Deterministic
/// for a given `n`.
pub fn synthetic_catalog(n: usize) -> Catalog {
    let mut catalog = Catalog::default();
    let mut rng = Lcg(0x5eed_ca7a_106u64);
    for i in 0..n {
        let owner = format!("owner{:03}", i % 250);
        let word = NAME_WORDS[i % NAME_WORDS.len()];
        let name = format!("{word}-{i:04}");
        let repo = match RepoRef::parse(&format!("{owner}/{name}")) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut entry = RepoEntry::new(repo.clone());
        entry.description = format!(
            "A {word} tool for {} work, volume {i} of the synthetic bench catalogue",
            TOPICS[(i / 3) % TOPICS.len()]
        );
        entry.stars = rng.below(50_000);
        entry.forks = entry.stars / 7;
        entry.language = LANGUAGES[i % LANGUAGES.len()].to_string();
        entry.license = LICENSES[(i / 3) % LICENSES.len()].to_string();
        let topic_count = 2 + (rng.below(2) as usize);
        for k in 0..topic_count {
            let topic = TOPICS[(i + k * 3) % TOPICS.len()].to_string();
            if !entry.topics.contains(&topic) {
                entry.topics.push(topic);
            }
        }
        if i % 5 != 0 {
            entry.latest_tag = format!("v{}.{}.{}", (i % 4) + 1, i % 12, i % 20);
        }
        if i % 10 < 3 {
            entry.installed_tag = if i % 20 < 3 {
                entry.latest_tag.clone()
            } else {
                "v1.0.0".to_string()
            };
        }
        entry.added_unix = 1_700_000_000 + i as u64;
        catalog.add(repo.clone());
        if let Some(slot) = catalog.get_mut(&repo) {
            *slot = entry;
        }
        if i % 5 == 0 {
            catalog.add_to_shelf("favourites", &repo);
        }
        if i % 7 == 0 {
            catalog.add_to_shelf("tools", &repo);
        }
    }
    catalog
}

/// A GitHub-shaped releases feed with `entries` releases, newest first.
pub fn synthetic_feed(entries: usize) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n  <title>Release notes from Bench</title>\n",
    );
    for k in 0..entries {
        let tag = format!("v{}.0.0", entries - k);
        out.push_str(&format!(
            "  <entry>\n    <id>tag:github.com,2008:Repository/1/{k}</id>\n    <updated>2026-11-01T00:00:00Z</updated>\n    <link rel=\"alternate\" type=\"text/html\" href=\"https://github.com/o/r/releases/tag/{tag}\"/>\n    <title>{tag}</title>\n    <content type=\"html\">&lt;p&gt;Bench release notes for {tag}&lt;/p&gt;</content>\n  </entry>\n"
        ));
    }
    out.push_str("</feed>");
    out
}

/// A fetcher that answers every request with the same feed body, so the
/// parallel check pipeline runs its real code (threads, parse, compare,
/// outcome assembly) with the network taken out of the measurement.
struct StubFetcher {
    body: String,
}

impl Fetcher for StubFetcher {
    fn get(
        &self,
        _url: &str,
        _etag: Option<&str>,
        _token: Option<&str>,
    ) -> Result<FetchResponse, String> {
        Ok(FetchResponse {
            status: 200,
            etag: String::new(),
            body: self.body.clone(),
        })
    }
}

/// Run `f` `iterations` times; return (median, min) in milliseconds.
fn median_time(iterations: usize, mut f: impl FnMut()) -> (f64, f64) {
    let mut samples: Vec<u128> = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        f();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let median = samples[samples.len() / 2] as f64 / 1_000_000.0;
    let min = samples[0] as f64 / 1_000_000.0;
    (median, min)
}

fn scenario(name: &str, detail: String, iterations: usize, times: (f64, f64)) -> BenchScenario {
    BenchScenario {
        name: name.to_string(),
        detail,
        iterations,
        median_ms: times.0,
        min_ms: times.1,
    }
}

/// Run every scenario against a synthetic catalogue of `repos` volumes.
/// Scratch files (the saved catalogue) go in `work_dir`, which the
/// caller owns and cleans up.
pub fn run_bench(repos: usize, work_dir: &Path) -> Result<BenchReport, String> {
    std::fs::create_dir_all(work_dir).map_err(|e| format!("bench folder: {e}"))?;
    let catalog = synthetic_catalog(repos);
    let path = work_dir.join("catalog.json");
    let mut scenarios = Vec::new();

    let save_times = median_time(5, || {
        let _ = catalog.save(&path);
    });
    scenarios.push(scenario(
        "catalogue save",
        format!("{repos} volumes written"),
        5,
        save_times,
    ));

    let catalog_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    let mut loaded: Option<Catalog> = None;
    let load_times = median_time(5, || {
        if let Ok(c) = Catalog::load(&path) {
            loaded = Some(c);
        }
    });
    scenarios.push(scenario(
        "catalogue load",
        format!("{repos} volumes, {} KB on disk", catalog_bytes / 1024),
        5,
        load_times,
    ));
    let catalog = match loaded {
        Some(c) => c,
        None => Catalog::load(&path)?,
    };

    let now = 1_763_000_000u64;
    let text_query = Query::parse("ledger");
    let mut text_hits = 0usize;
    let search_times = median_time(50, || {
        text_hits = search_execute(&catalog, &text_query, now).len();
    });
    scenarios.push(scenario(
        "search (text)",
        format!("\"ledger\" - {text_hits} hits"),
        50,
        search_times,
    ));

    let filtered_query = Query::parse("lang:rust stars:>500");
    let mut filtered_hits = 0usize;
    let filtered_times = median_time(50, || {
        filtered_hits = search_execute(&catalog, &filtered_query, now).len();
    });
    scenarios.push(scenario(
        "search (filtered)",
        format!("\"lang:rust stars:>500\" - {filtered_hits} hits"),
        50,
        filtered_times,
    ));

    let facet_times = median_time(50, || {
        let _ = catalog.language_facets();
        let _ = catalog.topic_facets();
        let _ = catalog.license_facets();
        let _ = catalog.owner_facets();
    });
    scenarios.push(scenario(
        "facet counts",
        "4 facet lists".into(),
        50,
        facet_times,
    ));

    let feed = synthetic_feed(30);
    let mut note_count = 0usize;
    let parse_times = median_time(200, || {
        note_count = parse_releases_atom(&feed).len();
    });
    scenarios.push(scenario(
        "feed parse",
        format!("30-entry feed - {note_count} releases parsed"),
        200,
        parse_times,
    ));

    // The release-check pipeline end to end, minus the network: the
    // first `batch` volumes, each told it runs v0.0.0 so the stub feed's
    // newer release registers as a change.
    let batch = repos.min(300);
    let entries: Vec<RepoEntry> = catalog
        .entries
        .iter()
        .take(batch)
        .map(|e| {
            let mut e = e.clone();
            e.latest_tag = "v0.0.0".to_string();
            e
        })
        .collect();
    let fetcher = StubFetcher { body: feed };
    for (label, concurrency) in [("release check x1", 1usize), ("release check x8", 8usize)] {
        let mut ok = 0usize;
        let times = median_time(3, || {
            let results = check_entries_parallel(&fetcher, &entries, now, concurrency, None);
            ok = results.iter().filter(|r| r.outcome.is_ok()).count();
        });
        let per_repo_us = if batch > 0 {
            times.0 * 1000.0 / batch as f64
        } else {
            0.0
        };
        scenarios.push(scenario(
            label,
            format!(
                "{batch} repos @ concurrency {} - {ok} ok, {per_repo_us:.0} us/repo",
                concurrency
            ),
            3,
            times,
        ));
    }

    Ok(BenchReport {
        repos,
        catalog_bytes,
        scenarios,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "repostacks-bench-test-{tag}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn synthetic_catalog_has_the_requested_size_and_shelves() {
        let catalog = synthetic_catalog(120);
        assert_eq!(catalog.entries.len(), 120);
        assert_eq!(catalog.shelf_members("favourites").len(), 24);
        assert!(!catalog.shelf_members("tools").is_empty());
        // Deterministic: a second build is entry-for-entry identical.
        let again = synthetic_catalog(120);
        assert_eq!(catalog.entries, again.entries);
    }

    #[test]
    fn synthetic_feed_parses_to_its_entry_count() {
        let notes = parse_releases_atom(&synthetic_feed(30));
        assert_eq!(notes.len(), 30);
        assert_eq!(notes[0].tag, "v30.0.0");
    }

    #[test]
    fn run_bench_reports_every_scenario() {
        let dir = scratch("run");
        let report = run_bench(60, &dir).expect("bench runs");
        assert_eq!(report.repos, 60);
        assert!(report.catalog_bytes > 0);
        assert_eq!(report.scenarios.len(), 8);
        for s in &report.scenarios {
            assert!(s.iterations > 0, "{} ran", s.name);
            assert!(s.median_ms >= 0.0, "{} timed", s.name);
        }
        let checks: Vec<&BenchScenario> = report
            .scenarios
            .iter()
            .filter(|s| s.name.starts_with("release check"))
            .collect();
        assert_eq!(checks.len(), 2);
        for c in checks {
            assert!(c.detail.contains("60 ok"), "{}: {}", c.name, c.detail);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
