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

pub fn check_releases<F: Fetcher>(
    fetcher: &F,
    entry: &mut RepoEntry,
    now_unix: u64,
) -> Result<CheckOutcome, String> {
    let url = entry.repo.releases_atom_url();
    let resp = fetcher.get(&url, None, None)?;
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
    let resp = fetcher.get(&entry.repo.api_repo_url(), etag, token)?;
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
    let resp = fetcher.get(&repo.api_release_by_tag_url(tag), None, token)?;
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
