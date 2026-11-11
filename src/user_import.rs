//! Importing a GitHub user's projects into the catalogue.
//!
//! One listing call per page (`GET /users/<name>/repos`, 100 per page)
//! carries enough metadata — description, stars, language, license,
//! topics — to catalogue a repo properly without a per-repo fetch, so
//! importing even a large library costs a handful of API calls, each
//! reported. Filters choose which of the listed repos land: owned by the
//! user or ones they are a member of, forks or original projects,
//! archived or not, public or private.

use crate::catalog::{Catalog, ImportReport, RepoEntry};
use crate::http::Fetcher;
use crate::json::{self, Json};
use crate::net::get_retry;
use crate::repo_ref::RepoRef;

/// One repository from a user's listing, with the metadata it carries.
#[derive(Debug, Clone, PartialEq)]
pub struct UserRepo {
    pub repo: RepoRef,
    pub owner: String,
    pub description: String,
    pub stars: u64,
    pub language: String,
    pub license: String,
    pub topics: Vec<String>,
    pub fork: bool,
    pub archived: bool,
    pub private: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    All,
    Owned,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forks {
    All,
    ForksOnly,
    SourcesOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Archived {
    /// Archived repos are left out (the default: a library of projects
    /// you still use).
    Exclude,
    Include,
    Only,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    All,
    PublicOnly,
    PrivateOnly,
}

/// Which of a user's listed repos to import. Defaults: everything the
/// user owns or belongs to, forks and originals alike, public and
/// private — except archived repos, which are left out unless asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserRepoFilter {
    pub ownership: Ownership,
    pub forks: Forks,
    pub archived: Archived,
    pub visibility: Visibility,
}

impl Default for UserRepoFilter {
    fn default() -> Self {
        UserRepoFilter {
            ownership: Ownership::All,
            forks: Forks::All,
            archived: Archived::Exclude,
            visibility: Visibility::All,
        }
    }
}

impl UserRepoFilter {
    pub fn accepts(&self, r: &UserRepo, username: &str) -> bool {
        let owned = r.owner.eq_ignore_ascii_case(username);
        match self.ownership {
            Ownership::All => {}
            Ownership::Owned if !owned => return false,
            Ownership::Member if owned => return false,
            _ => {}
        }
        match self.forks {
            Forks::All => {}
            Forks::ForksOnly if !r.fork => return false,
            Forks::SourcesOnly if r.fork => return false,
            _ => {}
        }
        match self.archived {
            Archived::Exclude if r.archived => return false,
            Archived::Only if !r.archived => return false,
            _ => {}
        }
        match self.visibility {
            Visibility::All => {}
            Visibility::PublicOnly if r.private => return false,
            Visibility::PrivateOnly if !r.private => return false,
            _ => {}
        }
        true
    }
}

/// Parse one entry of the listing response. Returns None for entries
/// without a usable `owner/name` (defensive; the listing is well-formed
/// in practice).
pub fn parse_user_repo(v: &Json) -> Option<UserRepo> {
    let full = v.get("full_name").and_then(Json::as_str)?;
    let repo = RepoRef::parse(full).ok()?;
    let owner = v
        .get("owner")
        .and_then(|o| o.get("login"))
        .and_then(Json::as_str)
        .unwrap_or(&repo.owner)
        .to_string();
    let license = v
        .get("license")
        .and_then(|l| l.get("spdx_id"))
        .and_then(Json::as_str)
        .unwrap_or("")
        .to_string();
    let topics = match v.get("topics") {
        Some(Json::Arr(items)) => items
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    Some(UserRepo {
        description: v
            .get("description")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
        stars: v
            .get("stargazers_count")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u64,
        language: v
            .get("language")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
        license,
        topics,
        fork: v.get("fork").and_then(Json::as_bool).unwrap_or(false),
        archived: v.get("archived").and_then(Json::as_bool).unwrap_or(false),
        private: v.get("private").and_then(Json::as_bool).unwrap_or(false),
        repo,
        owner,
    })
}

/// Fetch every repo in a user's listing, following pagination (100 per
/// page, at most 50 pages). Returns the repos and the number of API
/// calls spent — one per page. An anonymous listing shows public repos
/// only; a token also sees whatever private repos it may read.
pub fn fetch_user_repos<F: Fetcher>(
    fetcher: &F,
    username: &str,
    token: Option<&str>,
) -> Result<(Vec<UserRepo>, u32), String> {
    let username = username.trim().trim_start_matches('@');
    if username.is_empty()
        || username.len() > 39
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(format!("'{username}' does not look like a GitHub username"));
    }
    let mut all = Vec::new();
    let mut spent = 0u32;
    for page in 1..=50u32 {
        let url = format!(
            "https://api.github.com/users/{username}/repos?per_page=100&page={page}&sort=full_name"
        );
        let resp = get_retry(fetcher, &url, None, token)?;
        spent += 1;
        match resp.status {
            200 => {}
            404 => return Err(format!("GitHub user '{username}' was not found")),
            403 | 429 => {
                return Err(
                    "GitHub rate limit reached — release checks via Atom are unaffected, and a token raises the limit"
                        .into(),
                )
            }
            other => return Err(format!("listing {username}'s repos answered HTTP {other}")),
        }
        let v = json::parse(&resp.body)
            .map_err(|e| format!("{username}'s repo listing did not parse: {e}"))?;
        let items = v
            .as_arr()
            .ok_or_else(|| format!("{username}'s repo listing was not a list"))?;
        let n = items.len();
        for item in items {
            if let Some(r) = parse_user_repo(item) {
                all.push(r);
            }
        }
        if n < 100 {
            break;
        }
    }
    Ok((all, spent))
}

/// Apply an import: every listed repo that passes the filter and is not
/// already catalogued becomes an entry, pre-filled with the listing's
/// metadata (marked freshly fetched at `now_unix`, so the cache policy
/// will not immediately spend calls re-fetching it). With `dry_run`,
/// nothing is added but the report still says what would be.
pub fn apply_user_import(
    cat: &mut Catalog,
    repos: &[UserRepo],
    filter: &UserRepoFilter,
    username: &str,
    now_unix: u64,
    dry_run: bool,
) -> ImportReport {
    let mut report = ImportReport::default();
    for r in repos {
        if !filter.accepts(r, username) {
            continue;
        }
        if cat.get(&r.repo).is_some() {
            report.duplicates.push(r.repo.full_name());
            continue;
        }
        report.added.push(r.repo.full_name());
        if !dry_run {
            let mut e = RepoEntry::new(r.repo.clone());
            e.description = r.description.clone();
            e.stars = r.stars;
            e.language = r.language.clone();
            e.license = r.license.clone();
            e.topics = r.topics.clone();
            e.last_metadata_fetch = now_unix;
            e.added_unix = now_unix;
            cat.entries.push(e);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FetchResponse;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct PageFake {
        pages: Mutex<HashMap<u32, String>>,
        status: u16,
    }

    impl Fetcher for PageFake {
        fn get(
            &self,
            url: &str,
            _etag: Option<&str>,
            _token: Option<&str>,
        ) -> Result<FetchResponse, String> {
            if self.status != 200 {
                return Ok(FetchResponse {
                    status: self.status,
                    etag: String::new(),
                    body: String::new(),
                });
            }
            let page: u32 = url
                .split("&page=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            let body = self
                .pages
                .lock()
                .unwrap()
                .get(&page)
                .cloned()
                .unwrap_or_else(|| "[]".into());
            Ok(FetchResponse {
                status: 200,
                etag: String::new(),
                body,
            })
        }
    }

    fn repo_json(owner: &str, name: &str, extra: &str) -> String {
        format!(
            "{{\"full_name\":\"{owner}/{name}\",\"owner\":{{\"login\":\"{owner}\"}},\"description\":\"d\",\"stargazers_count\":7,\"language\":\"Rust\",\"license\":{{\"spdx_id\":\"MIT\"}},\"topics\":[\"cli\"],\"fork\":false,\"archived\":false,\"private\":false{extra}}}"
        )
    }

    fn listing(items: &[String]) -> String {
        format!("[{}]", items.join(","))
    }

    #[test]
    fn fetch_follows_pagination_and_counts_calls() {
        let page1: Vec<String> = (0..100)
            .map(|i| repo_json("someone", &format!("r{i:03}"), ""))
            .collect();
        let page2 = vec![
            repo_json("someone", "last-one", ""),
            repo_json("other-org", "member-repo", ""),
        ];
        let fake = PageFake {
            pages: Mutex::new(HashMap::from([(1, listing(&page1)), (2, listing(&page2))])),
            status: 200,
        };
        let (repos, spent) = fetch_user_repos(&fake, "someone", None).unwrap();
        assert_eq!(repos.len(), 102);
        assert_eq!(spent, 2);
        assert_eq!(repos[0].repo.full_name(), "someone/r000");
        assert_eq!(repos[101].owner, "other-org");
        assert_eq!(repos[0].license, "MIT");
        assert_eq!(repos[0].topics, vec!["cli".to_string()]);
    }

    #[test]
    fn fetch_reports_unknown_users_and_bad_names() {
        let fake = PageFake {
            pages: Mutex::new(HashMap::new()),
            status: 404,
        };
        assert!(fetch_user_repos(&fake, "ghost", None)
            .unwrap_err()
            .contains("not found"));
        let ok = PageFake {
            pages: Mutex::new(HashMap::new()),
            status: 200,
        };
        assert!(fetch_user_repos(&ok, "not a user!", None).is_err());
        assert!(fetch_user_repos(&ok, "", None).is_err());
    }

    fn sample() -> Vec<UserRepo> {
        let mk = |full: &str, owner: &str, fork: bool, archived: bool, private: bool| UserRepo {
            repo: RepoRef::parse(full).unwrap(),
            owner: owner.into(),
            description: String::new(),
            stars: 0,
            language: String::new(),
            license: String::new(),
            topics: Vec::new(),
            fork,
            archived,
            private,
        };
        vec![
            mk("me/own", "me", false, false, false),
            mk("me/forked", "me", true, false, false),
            mk("me/old", "me", false, true, false),
            mk("me/secret", "me", false, false, true),
            mk("org/team", "org", false, false, false),
        ]
    }

    fn names(repos: &[UserRepo], f: &UserRepoFilter, user: &str) -> Vec<String> {
        repos
            .iter()
            .filter(|r| f.accepts(r, user))
            .map(|r| r.repo.full_name())
            .collect()
    }

    #[test]
    fn filters_pick_the_promised_subsets() {
        let repos = sample();
        let d = UserRepoFilter::default();
        assert_eq!(
            names(&repos, &d, "me"),
            ["me/own", "me/forked", "me/secret", "org/team"]
        );
        let owned = UserRepoFilter {
            ownership: Ownership::Owned,
            ..d
        };
        assert_eq!(
            names(&repos, &owned, "me"),
            ["me/own", "me/forked", "me/secret"]
        );
        let member = UserRepoFilter {
            ownership: Ownership::Member,
            ..d
        };
        assert_eq!(names(&repos, &member, "me"), ["org/team"]);
        let sources = UserRepoFilter {
            forks: Forks::SourcesOnly,
            ..d
        };
        assert_eq!(
            names(&repos, &sources, "me"),
            ["me/own", "me/secret", "org/team"]
        );
        let forks = UserRepoFilter {
            forks: Forks::ForksOnly,
            ..d
        };
        assert_eq!(names(&repos, &forks, "me"), ["me/forked"]);
        let with_archived = UserRepoFilter {
            archived: Archived::Include,
            ..d
        };
        assert_eq!(names(&repos, &with_archived, "me").len(), 5);
        let only_archived = UserRepoFilter {
            archived: Archived::Only,
            ..d
        };
        assert_eq!(names(&repos, &only_archived, "me"), ["me/old"]);
        let public = UserRepoFilter {
            visibility: Visibility::PublicOnly,
            ..d
        };
        assert_eq!(
            names(&repos, &public, "me"),
            ["me/own", "me/forked", "org/team"]
        );
        let private = UserRepoFilter {
            visibility: Visibility::PrivateOnly,
            ..d
        };
        assert_eq!(names(&repos, &private, "me"), ["me/secret"]);
    }

    #[test]
    fn import_prefills_metadata_dedups_and_dry_run_is_free() {
        let repos = sample();
        let f = UserRepoFilter::default();
        let mut cat = Catalog::default();
        cat.add(RepoRef::parse("me/own").unwrap());
        let report = apply_user_import(&mut cat, &repos, &f, "me", 5000, false);
        assert_eq!(report.duplicates, ["me/own"]);
        assert_eq!(report.added.len(), 3);
        let e = cat.get(&RepoRef::parse("me/forked").unwrap()).unwrap();
        assert_eq!(e.last_metadata_fetch, 5000);
        assert_eq!(e.added_unix, 5000);
        // Second run: everything is a duplicate.
        let again = apply_user_import(&mut cat, &repos, &f, "me", 6000, false);
        assert!(again.added.is_empty());
        assert_eq!(again.duplicates.len(), 4);
        // Dry run reports without adding.
        let mut empty = Catalog::default();
        let preview = apply_user_import(&mut empty, &repos, &f, "me", 5000, true);
        assert_eq!(preview.added.len(), 4);
        assert!(empty.entries.is_empty());
    }
}
