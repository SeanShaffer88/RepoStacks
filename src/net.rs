//! Network operations: live release checks over Atom feeds and metadata
//! refresh over the REST API, both governed by `CachePolicy`.
//!
//! Quota accounting is explicit: every operation reports how many REST
//! units it spent under the current policy, so callers (and users) can see
//! the budget. Atom checks always spend zero.

use crate::atom::parse_releases_atom;
use crate::cache_policy::{Auth, CachePolicy};
use crate::catalog::RepoEntry;
use crate::http::Fetcher;
use crate::json;

#[derive(Debug, Clone, PartialEq)]
pub struct CheckOutcome {
    pub latest_tag: String,
    pub latest_title: String,
    pub release_count: usize,
    /// True when the feed's newest tag differs from the previously known one.
    pub changed: bool,
    pub quota_spent: u32,
}

/// One transport-level retry: a dropped connection or a curl hiccup is
/// worth exactly one second attempt. HTTP statuses are answers, not
/// transport failures, and are never retried here.
pub(crate) fn get_retry<F: Fetcher>(
    fetcher: &F,
    url: &str,
    etag: Option<&str>,
    token: Option<&str>,
) -> Result<crate::http::FetchResponse, String> {
    match fetcher.get(url, etag, token) {
        Ok(resp) => Ok(resp),
        Err(first) => fetcher.get(url, etag, token).map_err(|_| first),
    }
}

pub fn check_releases<F: Fetcher>(
    fetcher: &F,
    entry: &mut RepoEntry,
    now_unix: u64,
) -> Result<CheckOutcome, String> {
    let url = entry.repo.releases_atom_url();
    let resp = get_retry(fetcher, &url, None, None)?;
    if resp.status == 404 {
        return Err(format!(
            "{} has no releases feed (the repo may publish tags only, or not exist)",
            entry.repo
        ));
    }
    if resp.status != 200 {
        return Err(format!(
            "feed for {} answered HTTP {}",
            entry.repo, resp.status
        ));
    }
    let notes = parse_releases_atom(&resp.body);
    let (tag, title) = notes
        .first()
        .map(|n| (n.tag.clone(), n.title.clone()))
        .unwrap_or_default();
    let changed = !tag.is_empty() && tag != entry.latest_tag;
    if !tag.is_empty() {
        entry.latest_tag = tag.clone();
    }
    entry.last_release_check = now_unix;
    Ok(CheckOutcome {
        latest_tag: tag,
        latest_title: title,
        release_count: notes.len(),
        changed,
        quota_spent: 0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    Updated { quota_spent: u32 },
    NotModified { quota_spent: u32 },
    SkippedNotDue,
}

/// Refresh stars/description/language/license/topics from the REST API.
/// Anonymous callers are held to the policy interval unless `force` is set;
/// a token caller may refresh more often and pays nothing on a 304.
pub fn refresh_metadata<F: Fetcher>(
    fetcher: &F,
    entry: &mut RepoEntry,
    policy: &CachePolicy,
    token: Option<&str>,
    now_unix: u64,
    force: bool,
) -> Result<RefreshOutcome, String> {
    if !force
        && entry.last_metadata_fetch > 0
        && !policy.metadata_refresh_due(entry.last_metadata_fetch, now_unix)
    {
        return Ok(RefreshOutcome::SkippedNotDue);
    }
    let etag = if entry.etag.is_empty() {
        None
    } else {
        Some(entry.etag.as_str())
    };
    let resp = get_retry(fetcher, &entry.repo.api_repo_url(), etag, token)?;
    let quota_spent = if policy.response_counts_against_quota(resp.status) {
        1
    } else {
        0
    };
    match resp.status {
        304 => {
            entry.last_metadata_fetch = now_unix;
            Ok(RefreshOutcome::NotModified { quota_spent })
        }
        200 => {
            let v = json::parse(&resp.body).map_err(|e| format!("metadata for {} did not parse: {e}", entry.repo))?;
            entry.description = v.get("description").and_then(json::Json::as_str).unwrap_or("").to_string();
            entry.stars = v.get("stargazers_count").and_then(json::Json::as_f64).unwrap_or(0.0) as u64;
            entry.forks = v.get("forks_count").and_then(json::Json::as_f64).unwrap_or(0.0) as u64;
            entry.language = v.get("language").and_then(json::Json::as_str).unwrap_or("").to_string();
            if let Some(lic) = v.get("license") {
                entry.license = lic.get("spdx_id").and_then(json::Json::as_str).unwrap_or("").to_string();
            }
            if let Some(json::Json::Arr(items)) = v.get("topics") {
                entry.topics = items.iter().filter_map(json::Json::as_str).map(str::to_string).collect();
            }
            if !resp.etag.is_empty() {
                entry.etag = resp.etag.clone();
            }
            entry.last_metadata_fetch = now_unix;
            Ok(RefreshOutcome::Updated { quota_spent })
        }
        404 => Err(format!("{} was not found on GitHub", entry.repo)),
        403 | 429 => Err(format!(
            "GitHub rate limit reached ({} requests/hour for this mode) — release checks via Atom are unaffected",
            policy.auth.hourly_rest_limit()
        )),
        other => Err(format!("metadata for {} answered HTTP {other}", entry.repo)),
    }
}

/// The result of checking one entry in a parallel pass: the outcome plus
/// the checked copy of the entry (its `latest_tag` / `last_release_check`
/// already applied), so the caller can merge it back into the catalogue.
pub struct ParallelCheck {
    pub repo: crate::repo_ref::RepoRef,
    pub outcome: Result<CheckOutcome, String>,
    pub entry: RepoEntry,
}

/// Check many entries at once, at most `concurrency` fetches in flight.
/// Results come back in the input order. Each entry is checked on its own
/// clone, so no mutable state is shared between worker threads.
pub fn check_entries_parallel<F: Fetcher + Sync>(
    fetcher: &F,
    entries: &[RepoEntry],
    now_unix: u64,
    concurrency: usize,
) -> Vec<ParallelCheck> {
    let stride = concurrency.max(1);
    let mut out = Vec::with_capacity(entries.len());
    for chunk in entries.chunks(stride) {
        let results: Vec<ParallelCheck> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|e| {
                    scope.spawn(move || {
                        let mut entry = e.clone();
                        let outcome = check_releases(fetcher, &mut entry, now_unix);
                        ParallelCheck {
                            repo: entry.repo.clone(),
                            outcome,
                            entry,
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("check worker panicked"))
                .collect()
        });
        out.extend(results);
    }
    out
}

/// One entry's parallel metadata refresh: the outcome plus the entry
/// clone carrying the refreshed fields (or the untouched clone on error).
pub struct ParallelRefresh {
    pub repo: crate::repo_ref::RepoRef,
    pub outcome: Result<RefreshOutcome, String>,
    pub entry: RepoEntry,
}

/// Refresh many entries' metadata in parallel — the same chunked scoped
/// threads as `check_entries_parallel`. A sequential refresh of a large
/// catalogue costs one network round-trip per repo; this bounds the
/// wall-clock by the concurrency instead. Results keep input order.
#[allow(clippy::too_many_arguments)]
pub fn refresh_entries_parallel<F: Fetcher + Sync>(
    fetcher: &F,
    entries: &[RepoEntry],
    policy: &CachePolicy,
    token: Option<&str>,
    now_unix: u64,
    force: bool,
    concurrency: usize,
) -> Vec<ParallelRefresh> {
    let stride = concurrency.max(1);
    let mut out = Vec::with_capacity(entries.len());
    for chunk in entries.chunks(stride) {
        let results: Vec<ParallelRefresh> = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|e| {
                    scope.spawn(move || {
                        let mut entry = e.clone();
                        let outcome =
                            refresh_metadata(fetcher, &mut entry, policy, token, now_unix, force);
                        ParallelRefresh {
                            repo: entry.repo.clone(),
                            outcome,
                            entry,
                        }
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("refresh worker panicked"))
                .collect()
        });
        out.extend(results);
    }
    out
}

pub fn auth_for_token(token: Option<&str>) -> Auth {
    match token {
        Some(t) if !t.is_empty() => Auth::Token,
        _ => Auth::Anonymous,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FetchResponse;
    use crate::repo_ref::RepoRef;
    use std::cell::RefCell;

    struct Fake {
        responses: RefCell<Vec<FetchResponse>>,
        calls: RefCell<Vec<String>>,
    }
    impl Fake {
        fn new(responses: Vec<FetchResponse>) -> Fake {
            Fake {
                responses: RefCell::new(responses),
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl Fetcher for Fake {
        fn get(
            &self,
            url: &str,
            _etag: Option<&str>,
            _token: Option<&str>,
        ) -> Result<FetchResponse, String> {
            self.calls.borrow_mut().push(url.to_string());
            Ok(self.responses.borrow_mut().remove(0))
        }
    }

    const FEED: &str = "<feed><entry><updated>2026-11-01T00:00:00Z</updated><link rel=\"alternate\" href=\"https://github.com/o/r/releases/tag/v2\"/><title>v2</title></entry></feed>";

    fn entry() -> RepoEntry {
        RepoEntry::new(RepoRef::parse("o/r").unwrap())
    }

    #[test]
    fn atom_check_detects_new_tag_for_free() {
        let fake = Fake::new(vec![FetchResponse {
            status: 200,
            etag: String::new(),
            body: FEED.into(),
        }]);
        let mut e = entry();
        let out = check_releases(&fake, &mut e, 500).unwrap();
        assert!(out.changed);
        assert_eq!(out.quota_spent, 0);
        assert_eq!(e.latest_tag, "v2");
        assert_eq!(e.last_release_check, 500);
        // Second check with the same feed: no change.
        let fake2 = Fake::new(vec![FetchResponse {
            status: 200,
            etag: String::new(),
            body: FEED.into(),
        }]);
        let out2 = check_releases(&fake2, &mut e, 600).unwrap();
        assert!(!out2.changed);
    }

    #[test]
    fn metadata_refresh_updates_fields_and_stores_etag() {
        let body = r#"{"description":"A thing","stargazers_count":42,"language":"Rust","license":{"spdx_id":"MIT"},"topics":["cli","rust"]}"#;
        let fake = Fake::new(vec![FetchResponse {
            status: 200,
            etag: "\"x\"".into(),
            body: body.into(),
        }]);
        let mut e = entry();
        let policy = CachePolicy::for_auth(Auth::Anonymous);
        let out = refresh_metadata(&fake, &mut e, &policy, None, 1_000, false).unwrap();
        assert_eq!(out, RefreshOutcome::Updated { quota_spent: 1 });
        assert_eq!(e.description, "A thing");
        assert_eq!(e.stars, 42);
        assert_eq!(e.license, "MIT");
        assert_eq!(e.topics, vec!["cli".to_string(), "rust".to_string()]);
        assert_eq!(e.etag, "\"x\"");
        // Not due again right away for an anonymous caller.
        let out2 = refresh_metadata(&fake, &mut e, &policy, None, 2_000, false).unwrap();
        assert_eq!(out2, RefreshOutcome::SkippedNotDue);
    }

    #[test]
    fn release_assets_parse_with_digest() {
        let body = r#"{"tag_name":"v1","assets":[{"name":"app-win.zip","browser_download_url":"https://example.com/app-win.zip","size":1234,"digest":"sha256:beef"},{"name":"","browser_download_url":"x"}]}"#;
        let fake = Fake::new(vec![FetchResponse {
            status: 200,
            etag: String::new(),
            body: body.into(),
        }]);
        let repo = RepoRef::parse("o/r").unwrap();
        let (assets, spent) = fetch_release_assets(&fake, &repo, "v1", None).unwrap();
        assert_eq!(spent, 1);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].name, "app-win.zip");
        assert_eq!(assets[0].digest, "sha256:beef");
    }

    struct FlakyFake {
        attempts: std::sync::Mutex<usize>,
    }
    impl Fetcher for FlakyFake {
        fn get(
            &self,
            _url: &str,
            _etag: Option<&str>,
            _token: Option<&str>,
        ) -> Result<FetchResponse, String> {
            let mut n = self.attempts.lock().unwrap();
            *n += 1;
            if *n == 1 {
                Err("connection dropped".into())
            } else {
                Ok(FetchResponse {
                    status: 200,
                    etag: String::new(),
                    body: FEED.into(),
                })
            }
        }
    }

    #[test]
    fn transport_failure_is_retried_once() {
        let fake = FlakyFake {
            attempts: std::sync::Mutex::new(0),
        };
        let mut e = entry();
        let out = check_releases(&fake, &mut e, 700).unwrap();
        assert!(out.changed);
        assert_eq!(*fake.attempts.lock().unwrap(), 2);
    }

    struct MapFake {
        by_url: std::sync::Mutex<std::collections::HashMap<String, FetchResponse>>,
    }
    impl Fetcher for MapFake {
        fn get(
            &self,
            url: &str,
            _etag: Option<&str>,
            _token: Option<&str>,
        ) -> Result<FetchResponse, String> {
            self.by_url
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| format!("no scripted response for {url}"))
        }
    }

    #[test]
    fn parallel_checks_cover_all_entries_in_order() {
        let mut by_url = std::collections::HashMap::new();
        let mut entries = Vec::new();
        for name in ["a/one", "b/two", "c/three"] {
            let e = RepoEntry::new(RepoRef::parse(name).unwrap());
            by_url.insert(
                e.repo.releases_atom_url(),
                FetchResponse {
                    status: 200,
                    etag: String::new(),
                    body: FEED.into(),
                },
            );
            entries.push(e);
        }
        let fake = MapFake {
            by_url: std::sync::Mutex::new(by_url),
        };
        let results = check_entries_parallel(&fake, &entries, 900, 2);
        assert_eq!(results.len(), 3);
        for (i, name) in ["a/one", "b/two", "c/three"].iter().enumerate() {
            assert_eq!(results[i].repo.full_name(), *name);
            assert!(results[i].outcome.as_ref().unwrap().changed);
            assert_eq!(results[i].entry.latest_tag, "v2");
            assert_eq!(results[i].entry.last_release_check, 900);
        }
    }

    #[test]
    fn anonymous_304_costs_quota_but_token_304_does_not() {
        for (auth, expected) in [(Auth::Anonymous, 1), (Auth::Token, 0)] {
            let fake = Fake::new(vec![FetchResponse {
                status: 304,
                etag: String::new(),
                body: String::new(),
            }]);
            let mut e = entry();
            let policy = CachePolicy::for_auth(auth);
            let out = refresh_metadata(&fake, &mut e, &policy, None, 1_000, true).unwrap();
            assert_eq!(
                out,
                RefreshOutcome::NotModified {
                    quota_spent: expected
                },
                "auth {auth:?}"
            );
        }
    }
}

/// One downloadable file on a release, as the REST API describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256:<hex>` when GitHub publishes a digest for the asset.
    pub digest: String,
}

/// Fetch a release's asset list — the one REST call a new release costs.
/// Returns the assets and the quota spent (1).
pub fn fetch_release_assets<F: Fetcher>(
    fetcher: &F,
    repo: &crate::repo_ref::RepoRef,
    tag: &str,
    token: Option<&str>,
) -> Result<(Vec<ReleaseAsset>, u32), String> {
    let resp = get_retry(fetcher, &repo.api_release_by_tag_url(tag), None, token)?;
    match resp.status {
        200 => {}
        404 => return Err(format!("{repo} has no release tagged {tag}")),
        403 | 429 => {
            return Err(
                "GitHub rate limit reached — try again after the hour resets, or add a token"
                    .into(),
            )
        }
        other => return Err(format!("release {tag} for {repo} answered HTTP {other}")),
    }
    let v =
        json::parse(&resp.body).map_err(|e| format!("release for {repo} did not parse: {e}"))?;
    let mut assets = Vec::new();
    if let Some(json::Json::Arr(items)) = v.get("assets") {
        for a in items {
            let name = a
                .get("name")
                .and_then(json::Json::as_str)
                .unwrap_or("")
                .to_string();
            let url = a
                .get("browser_download_url")
                .and_then(json::Json::as_str)
                .unwrap_or("")
                .to_string();
            if name.is_empty() || url.is_empty() {
                continue;
            }
            assets.push(ReleaseAsset {
                name,
                url,
                size: a.get("size").and_then(json::Json::as_f64).unwrap_or(0.0) as u64,
                digest: a
                    .get("digest")
                    .and_then(json::Json::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok((assets, 1))
}
