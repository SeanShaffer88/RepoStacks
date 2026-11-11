//! The search engine — built to be as capable as the catalogue deserves.
//!
//! Query language (space-separated terms, all must match):
//! - bare words: matched against name, owner, description, language, topics
//! - `owner:NAME`, `lang:NAME` / `language:NAME`, `topic:NAME`,
//!   `license:NAME`, `shelf:NAME`
//! - `stars:>1000`, `stars:<500`, `stars:100..5000`, `stars:>=42`
//! - `tag:TEXT` matches the latest or installed release tag
//! - `has:release`, `has:installed`, `has:update`, `has:auto` (standing order)
//! - `is:installed`, `is:missing`, `is:current`, `is:outdated`
//! - `is:due` (not release-checked in over a week), `is:never` (never checked)
//! - any term negated with a leading `-`: `-lang:c`, `-topic:games`, `-cli`
//! - `sort:name|stars|release|checked|added` and `sort:-stars` for
//!   descending
//! - `limit:N`
//! Unknown `key:value` terms fall back to a plain text match, so a typo
//! narrows results instead of erroring the whole search.

use crate::catalog::{Catalog, RepoEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Stars,
    ReleaseChecked,
    Added,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub text_terms: Vec<String>,
    pub owner: String,
    pub language: String,
    pub topic: String,
    pub license: String,
    pub shelf: String,
    pub tag: String,
    pub stars_min: Option<u64>,
    pub stars_max: Option<u64>,
    pub has_release: bool,
    pub has_installed: bool,
    pub has_update: bool,
    pub has_auto: bool,
    pub installed: Option<bool>,
    pub outdated: Option<bool>,
    pub due_check: bool,
    pub never_checked: bool,
    pub negated: Vec<NegTerm>,
    pub sort: Option<(SortKey, bool)>, // (key, descending)
    pub limit: Option<usize>,
}

/// One negated term (`-lang:c`, `-topic:games`, `-is:installed`, `-cli`):
/// an entry that matches it is excluded from the results.
#[derive(Debug, Clone, PartialEq)]
pub enum NegTerm {
    Text(String),
    Owner(String),
    Language(String),
    Topic(String),
    License(String),
    Shelf(String),
    Tag(String),
    HasRelease,
    HasInstalled,
    HasUpdate,
    HasAuto,
    Installed(bool),
    Outdated(bool),
    DueCheck,
    NeverChecked,
    Stars { min: Option<u64>, max: Option<u64> },
}

impl Default for Query {
    fn default() -> Query {
        Query {
            text_terms: Vec::new(),
            owner: String::new(),
            language: String::new(),
            topic: String::new(),
            license: String::new(),
            shelf: String::new(),
            tag: String::new(),
            stars_min: None,
            stars_max: None,
            has_release: false,
            has_installed: false,
            has_update: false,
            has_auto: false,
            installed: None,
            outdated: None,
            due_check: false,
            never_checked: false,
            negated: Vec::new(),
            sort: None,
            limit: None,
        }
    }
}

impl Query {
    pub fn parse(input: &str) -> Query {
        let mut q = Query::default();
        for raw in tokenize(input) {
            let term = raw.trim();
            if term.is_empty() {
                continue;
            }
            // A leading '-' negates the term: -lang:c, -topic:games,
            // -is:installed, -cli. (sort:-stars is a value, not a negation.)
            if let Some(rest) = term.strip_prefix('-') {
                if !rest.is_empty() {
                    if let Some(neg) = parse_negated(rest) {
                        q.negated.push(neg);
                        continue;
                    }
                }
            }
            let Some((key, value)) = term.split_once(':') else {
                q.text_terms.push(term.to_lowercase());
                continue;
            };
            let value = value.trim();
            match key.to_lowercase().as_str() {
                "owner" => q.owner = value.to_lowercase(),
                "lang" | "language" => q.language = value.to_lowercase(),
                "topic" => q.topic = value.to_lowercase(),
                "license" => q.license = value.to_lowercase(),
                "shelf" => q.shelf = value.to_string(),
                "tag" => q.tag = value.to_lowercase(),
                "stars" => parse_stars(value, &mut q),
                "has" => match value.to_lowercase().as_str() {
                    "release" | "releases" => q.has_release = true,
                    "installed" => q.has_installed = true,
                    "update" | "updates" => q.has_update = true,
                    "auto" | "standing-order" => q.has_auto = true,
                    _ => q.text_terms.push(term.to_lowercase()),
                },
                "is" => match value.to_lowercase().as_str() {
                    "installed" => q.installed = Some(true),
                    "missing" | "notinstalled" | "not-installed" => q.installed = Some(false),
                    "current" | "uptodate" | "up-to-date" => q.outdated = Some(false),
                    "outdated" | "stale" => q.outdated = Some(true),
                    "due" => q.due_check = true,
                    "never" => q.never_checked = true,
                    _ => q.text_terms.push(term.to_lowercase()),
                },
                "sort" => {
                    let (desc, name) = match value.strip_prefix('-') {
                        Some(rest) => (true, rest),
                        None => (false, value),
                    };
                    q.sort = match name.to_lowercase().as_str() {
                        "name" | "title" => Some((SortKey::Name, desc)),
                        "stars" => Some((SortKey::Stars, desc)),
                        "release" | "checked" => Some((SortKey::ReleaseChecked, desc)),
                        "added" => Some((SortKey::Added, desc)),
                        _ => None,
                    };
                }
                "limit" => q.limit = value.parse::<usize>().ok(),
                _ => q.text_terms.push(term.to_lowercase()),
            }
        }
        q
    }
}

/// Split on whitespace, keeping double-quoted spans together so values
/// with spaces work: `shelf:"AI & LLMs"`, `"card catalogue"`.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in input.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn stars_range(value: &str) -> (Option<u64>, Option<u64>) {
    let v = value.replace(',', "");
    let (mut min, mut max) = (None, None);
    if let Some((lo, hi)) = v.split_once("..") {
        min = lo.parse().ok();
        max = hi.parse().ok();
    } else if let Some(n) = v.strip_prefix(">=") {
        min = n.parse().ok();
    } else if let Some(n) = v.strip_prefix('>') {
        min = n.parse::<u64>().ok().map(|x| x + 1);
    } else if let Some(n) = v.strip_prefix("<=") {
        max = n.parse().ok();
    } else if let Some(n) = v.strip_prefix('<') {
        max = n.parse::<u64>().ok().map(|x| x.saturating_sub(1));
    } else if let Ok(n) = v.parse::<u64>() {
        min = Some(n);
        max = Some(n);
    }
    (min, max)
}

fn parse_stars(value: &str, q: &mut Query) {
    let (min, max) = stars_range(value);
    q.stars_min = min;
    q.stars_max = max;
}

/// Parse the part after a leading '-' into a negated term. A bare word
/// negates as a text term; `sort:`/`limit:` cannot be negated.
fn parse_negated(rest: &str) -> Option<NegTerm> {
    let Some((key, value)) = rest.split_once(':') else {
        return Some(NegTerm::Text(rest.to_lowercase()));
    };
    let value = value.trim();
    let lv = value.to_lowercase();
    Some(match key.to_lowercase().as_str() {
        "owner" => NegTerm::Owner(lv),
        "lang" | "language" => NegTerm::Language(lv),
        "topic" => NegTerm::Topic(lv),
        "license" => NegTerm::License(lv),
        "shelf" => NegTerm::Shelf(value.to_string()),
        "tag" => NegTerm::Tag(lv),
        "stars" => {
            let (min, max) = stars_range(value);
            NegTerm::Stars { min, max }
        }
        "has" => match lv.as_str() {
            "release" | "releases" => NegTerm::HasRelease,
            "installed" => NegTerm::HasInstalled,
            "update" | "updates" => NegTerm::HasUpdate,
            "auto" | "standing-order" => NegTerm::HasAuto,
            _ => return None,
        },
        "is" => match lv.as_str() {
            "installed" => NegTerm::Installed(true),
            "missing" | "notinstalled" | "not-installed" => NegTerm::Installed(false),
            "current" | "uptodate" | "up-to-date" => NegTerm::Outdated(false),
            "outdated" | "stale" => NegTerm::Outdated(true),
            "due" => NegTerm::DueCheck,
            "never" => NegTerm::NeverChecked,
            _ => return None,
        },
        _ => return None,
    })
}

/// A week without a release check makes a volume "due" — the Librarian's
/// default Atom interval is far shorter, so a due volume was missed.
pub const DUE_CHECK_SECS: u64 = 7 * 86_400;

fn is_due(e: &RepoEntry, now_unix: u64) -> bool {
    e.last_release_check == 0 || now_unix.saturating_sub(e.last_release_check) > DUE_CHECK_SECS
}

/// Run a query over the catalogue at a moment in time (the clock only
/// matters for `is:due`). Shelf membership comes from the catalogue's
/// shelf map, so `shelf:` works without duplicating data.
pub fn execute<'a>(catalog: &'a Catalog, q: &Query, now_unix: u64) -> Vec<&'a RepoEntry> {
    let shelf_members: Vec<String> = if q.shelf.is_empty() {
        Vec::new()
    } else {
        catalog
            .shelves
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&q.shelf))
            .map(|(_, members)| members.clone())
            .unwrap_or_default()
    };
    // Member lists for negated shelf terms (-shelf:Games).
    let neg_shelves: Vec<Vec<String>> = q
        .negated
        .iter()
        .filter_map(|n| match n {
            NegTerm::Shelf(name) => Some(
                catalog
                    .shelves
                    .iter()
                    .find(|(s, _)| s.eq_ignore_ascii_case(name))
                    .map(|(_, members)| members.clone())
                    .unwrap_or_default(),
            ),
            _ => None,
        })
        .collect();
    let has_neg_text = q.negated.iter().any(|n| matches!(n, NegTerm::Text(_)));
    let mut hits: Vec<&RepoEntry> = catalog
        .entries
        .iter()
        .filter(|e| {
            if !q.owner.is_empty() && !e.repo.owner.to_lowercase().contains(&q.owner) {
                return false;
            }
            if !q.language.is_empty() && !e.language.to_lowercase().contains(&q.language) {
                return false;
            }
            if !q.license.is_empty() && !e.license.to_lowercase().contains(&q.license) {
                return false;
            }
            if !q.topic.is_empty() && !e.topics.iter().any(|t| t.to_lowercase().contains(&q.topic))
            {
                return false;
            }
            if !q.shelf.is_empty() && !shelf_members.contains(&e.repo.full_name()) {
                return false;
            }
            if !q.tag.is_empty()
                && !e.latest_tag.to_lowercase().contains(&q.tag)
                && !e.installed_tag.to_lowercase().contains(&q.tag)
            {
                return false;
            }
            if let Some(min) = q.stars_min {
                if e.stars < min {
                    return false;
                }
            }
            if let Some(max) = q.stars_max {
                if e.stars > max {
                    return false;
                }
            }
            if q.has_release && e.latest_tag.is_empty() {
                return false;
            }
            if q.has_installed && !e.is_installed() {
                return false;
            }
            if q.has_update && !e.update_available() {
                return false;
            }
            if q.has_auto && !e.auto_download {
                return false;
            }
            if let Some(want) = q.installed {
                if e.is_installed() != want {
                    return false;
                }
            }
            if let Some(want_outdated) = q.outdated {
                // "Outdated" only applies to installed volumes.
                if !e.is_installed() || e.update_available() != want_outdated {
                    return false;
                }
            }
            if q.due_check && !is_due(e, now_unix) {
                return false;
            }
            if q.never_checked && e.last_release_check != 0 {
                return false;
            }
            // The haystack is built once per entry, not once per term —
            // and only when a text term (positive or negated) needs it.
            let mut hay: Option<String> = None;
            if !q.text_terms.is_empty() || has_neg_text {
                hay = Some(
                    format!(
                        "{} {} {} {} {} {}",
                        e.repo.full_name(),
                        e.description,
                        e.language,
                        e.license,
                        e.topics.join(" "),
                        e.latest_tag
                    )
                    .to_lowercase(),
                );
            }
            if let Some(hay) = &hay {
                for term in &q.text_terms {
                    if !hay.contains(term) {
                        return false;
                    }
                }
            }
            // Negated terms: matching any one excludes the entry.
            let mut neg_shelf_idx = 0;
            for n in &q.negated {
                let excluded = match n {
                    NegTerm::Text(t) => hay.as_ref().map(|h| h.contains(t)).unwrap_or(false),
                    NegTerm::Owner(v) => e.repo.owner.to_lowercase().contains(v),
                    NegTerm::Language(v) => e.language.to_lowercase().contains(v),
                    NegTerm::License(v) => e.license.to_lowercase().contains(v),
                    NegTerm::Topic(v) => e.topics.iter().any(|t| t.to_lowercase().contains(v)),
                    NegTerm::Shelf(_) => {
                        let members = &neg_shelves[neg_shelf_idx];
                        neg_shelf_idx += 1;
                        members.contains(&e.repo.full_name())
                    }
                    NegTerm::Tag(v) => {
                        e.latest_tag.to_lowercase().contains(v)
                            || e.installed_tag.to_lowercase().contains(v)
                    }
                    NegTerm::HasRelease => !e.latest_tag.is_empty(),
                    NegTerm::HasInstalled => e.is_installed(),
                    NegTerm::HasUpdate => e.update_available(),
                    NegTerm::HasAuto => e.auto_download,
                    NegTerm::Installed(want) => e.is_installed() == *want,
                    NegTerm::Outdated(want) => e.is_installed() && e.update_available() == *want,
                    NegTerm::DueCheck => is_due(e, now_unix),
                    NegTerm::NeverChecked => e.last_release_check == 0,
                    NegTerm::Stars { min, max } => {
                        min.map(|m| e.stars >= m).unwrap_or(true)
                            && max.map(|m| e.stars <= m).unwrap_or(true)
                    }
                };
                if excluded {
                    return false;
                }
            }
            true
        })
        .collect();
    if let Some((key, desc)) = q.sort {
        hits.sort_by(|a, b| {
            let ord = match key {
                SortKey::Name => a
                    .repo
                    .full_name()
                    .to_lowercase()
                    .cmp(&b.repo.full_name().to_lowercase()),
                SortKey::Stars => a.stars.cmp(&b.stars),
                SortKey::ReleaseChecked => a.last_release_check.cmp(&b.last_release_check),
                SortKey::Added => a.added_unix.cmp(&b.added_unix).then_with(|| {
                    a.repo
                        .full_name()
                        .to_lowercase()
                        .cmp(&b.repo.full_name().to_lowercase())
                }),
            };
            if desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }
    if let Some(limit) = q.limit {
        hits.truncate(limit);
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_ref::RepoRef;

    fn cat() -> Catalog {
        let mut c = Catalog::default();
        for (name, lang, lic, stars, topics) in [
            ("rust-lang/rust", "Rust", "MIT", 97_000, vec!["compiler"]),
            ("golang/go", "Go", "BSD-3-Clause", 122_000, vec!["compiler"]),
            ("ollama/ollama", "Go", "MIT", 95_000, vec!["ai", "llm"]),
            ("tauri-apps/tauri", "Rust", "MIT", 72_000, vec!["desktop"]),
        ] {
            c.add(RepoRef::parse(name).unwrap());
            let e = c.get_mut(&RepoRef::parse(name).unwrap()).unwrap();
            e.language = lang.into();
            e.license = lic.into();
            e.stars = stars;
            e.topics = topics.into_iter().map(str::to_string).collect();
        }
        let r = RepoRef::parse("ollama/ollama").unwrap();
        let e = c.get_mut(&r).unwrap();
        e.latest_tag = "v0.3.14".into();
        e.installed_tag = "v0.3.12".into();
        e.auto_download = true;
        c.add_to_shelf("AI & LLMs", &r);
        c
    }

    #[test]
    fn field_filters_and_ranges() {
        let c = cat();
        assert_eq!(execute(&c, &Query::parse("lang:rust"), 0).len(), 2);
        assert_eq!(
            execute(&c, &Query::parse("topic:compiler stars:>100000"), 0).len(),
            1
        );
        assert_eq!(
            execute(&c, &Query::parse("stars:90000..100000"), 0).len(),
            2
        );
        assert_eq!(execute(&c, &Query::parse("license:mit"), 0).len(), 3);
        assert_eq!(execute(&c, &Query::parse("owner:rust"), 0).len(), 1);
        assert_eq!(
            execute(&c, &Query::parse("shelf:\"AI & LLMs\""), 0).len(),
            1
        );
        assert_eq!(execute(&c, &Query::parse("shelf:Nope"), 0).len(), 0);
    }

    #[test]
    fn status_filters_and_text() {
        let c = cat();
        assert_eq!(execute(&c, &Query::parse("has:update"), 0).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:outdated"), 0).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:installed"), 0).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:missing"), 0).len(), 3);
        assert_eq!(execute(&c, &Query::parse("has:auto"), 0).len(), 1);
        assert_eq!(execute(&c, &Query::parse("tag:v0.3"), 0).len(), 1);
        assert_eq!(
            execute(&c, &Query::parse("tag:0.3.12"), 0).len(),
            1,
            "installed tag matches too"
        );
        assert_eq!(execute(&c, &Query::parse("llm"), 0).len(), 1);
        assert_eq!(
            execute(&c, &Query::parse("compiler sort:-stars"), 0)
                .first()
                .unwrap()
                .repo
                .full_name(),
            "golang/go"
        );
        // sort:added uses the real added stamp, newest first with -added.
        let mut c2 = cat();
        for (name, when) in [
            ("rust-lang/rust", 100),
            ("golang/go", 300),
            ("ollama/ollama", 200),
            ("tauri-apps/tauri", 50),
        ] {
            c2.get_mut(&RepoRef::parse(name).unwrap())
                .unwrap()
                .added_unix = when;
        }
        assert_eq!(
            execute(&c2, &Query::parse("sort:-added limit:1"), 0)
                .first()
                .unwrap()
                .repo
                .full_name(),
            "golang/go"
        );
        // sort:stars is ascending (lowest first); sort:-stars descends.
        assert_eq!(
            execute(&c, &Query::parse("sort:stars limit:1"), 0)
                .first()
                .unwrap()
                .repo
                .full_name(),
            "tauri-apps/tauri"
        );
        assert_eq!(
            execute(&c, &Query::parse("sort:-stars limit:1"), 0)
                .first()
                .unwrap()
                .repo
                .full_name(),
            "golang/go"
        );
    }

    #[test]
    fn negation_excludes_matches() {
        let c = cat();
        assert_eq!(execute(&c, &Query::parse("-lang:rust"), 0).len(), 2);
        assert_eq!(
            execute(&c, &Query::parse("lang:rust -owner:tauri"), 0).len(),
            1
        );
        assert_eq!(execute(&c, &Query::parse("-is:installed"), 0).len(), 3);
        assert_eq!(execute(&c, &Query::parse("-llm"), 0).len(), 3);
        assert_eq!(
            execute(&c, &Query::parse("compiler -topic:compiler"), 0).len(),
            0
        );
        assert_eq!(execute(&c, &Query::parse("-stars:>100000"), 0).len(), 3);
        assert_eq!(
            execute(&c, &Query::parse("-shelf:\"AI & LLMs\""), 0).len(),
            3
        );
        // sort:-stars is a sort value, never a negation.
        assert!(Query::parse("sort:-stars").negated.is_empty());
    }

    #[test]
    fn due_and_never_need_the_clock() {
        let now = 1_000_000u64;
        let mut c = cat(); // fixture entries were never checked (stamp 0)
        assert_eq!(execute(&c, &Query::parse("is:due"), now).len(), 4);
        assert_eq!(execute(&c, &Query::parse("is:never"), now).len(), 4);
        let r = RepoRef::parse("rust-lang/rust").unwrap();
        c.get_mut(&r).unwrap().last_release_check = now;
        assert_eq!(execute(&c, &Query::parse("is:due"), now).len(), 3);
        assert_eq!(execute(&c, &Query::parse("is:never"), now).len(), 3);
        assert_eq!(execute(&c, &Query::parse("-is:due"), now).len(), 1);
    }
}
