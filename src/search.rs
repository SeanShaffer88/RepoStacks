//! The search engine — built to be as capable as the catalogue deserves.
//!
//! Query language (space-separated terms, all must match):
//! - bare words: matched against name, owner, description, language, topics
//! - `owner:NAME`, `lang:NAME` / `language:NAME`, `topic:NAME`,
//!   `license:NAME`, `shelf:NAME`
//! - `stars:>1000`, `stars:<500`, `stars:100..5000`, `stars:>=42`
//! - `has:release`, `has:installed`, `has:update`, `has:auto` (standing order)
//! - `is:installed`, `is:missing`, `is:current`, `is:outdated`
//! - `sort:name|stars|release|checked|added` and `sort:-stars` for descending
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
    pub stars_min: Option<u64>,
    pub stars_max: Option<u64>,
    pub has_release: bool,
    pub has_installed: bool,
    pub has_update: bool,
    pub has_auto: bool,
    pub installed: Option<bool>,
    pub outdated: Option<bool>,
    pub sort: Option<(SortKey, bool)>, // (key, descending)
    pub limit: Option<usize>,
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
            stars_min: None,
            stars_max: None,
            has_release: false,
            has_installed: false,
            has_update: false,
            has_auto: false,
            installed: None,
            outdated: None,
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

fn parse_stars(value: &str, q: &mut Query) {
    let v = value.replace(',', "");
    if let Some((lo, hi)) = v.split_once("..") {
        q.stars_min = lo.parse().ok();
        q.stars_max = hi.parse().ok();
    } else if let Some(n) = v.strip_prefix(">=") {
        q.stars_min = n.parse().ok();
    } else if let Some(n) = v.strip_prefix('>') {
        q.stars_min = n.parse::<u64>().ok().map(|x| x + 1);
    } else if let Some(n) = v.strip_prefix("<=") {
        q.stars_max = n.parse().ok();
    } else if let Some(n) = v.strip_prefix('<') {
        q.stars_max = n.parse::<u64>().ok().map(|x| x.saturating_sub(1));
    } else if let Ok(n) = v.parse::<u64>() {
        q.stars_min = Some(n);
        q.stars_max = Some(n);
    }
}

/// Run a query over the catalogue. Shelf membership comes from the
/// catalogue's shelf map, so `shelf:` works without duplicating data.
pub fn execute<'a>(catalog: &'a Catalog, q: &Query) -> Vec<&'a RepoEntry> {
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
            for term in &q.text_terms {
                let hay = format!(
                    "{} {} {} {} {} {}",
                    e.repo.full_name(),
                    e.description,
                    e.language,
                    e.license,
                    e.topics.join(" "),
                    e.latest_tag
                )
                .to_lowercase();
                if !hay.contains(term) {
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
                SortKey::Added => a
                    .repo
                    .full_name()
                    .to_lowercase()
                    .cmp(&b.repo.full_name().to_lowercase()),
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
        assert_eq!(execute(&c, &Query::parse("lang:rust")).len(), 2);
        assert_eq!(
            execute(&c, &Query::parse("topic:compiler stars:>100000")).len(),
            1
        );
        assert_eq!(execute(&c, &Query::parse("stars:90000..100000")).len(), 2);
        assert_eq!(execute(&c, &Query::parse("license:mit")).len(), 3);
        assert_eq!(execute(&c, &Query::parse("owner:rust")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("shelf:\"AI & LLMs\"")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("shelf:Nope")).len(), 0);
    }

    #[test]
    fn status_filters_and_text() {
        let c = cat();
        assert_eq!(execute(&c, &Query::parse("has:update")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:outdated")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:installed")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("is:missing")).len(), 3);
        assert_eq!(execute(&c, &Query::parse("has:auto")).len(), 1);
        assert_eq!(execute(&c, &Query::parse("llm")).len(), 1);
        assert_eq!(
            execute(&c, &Query::parse("compiler sort:-stars"))
                .first()
                .unwrap()
                .repo
                .full_name(),
            "golang/go"
        );
        // sort:stars is ascending (lowest first); sort:-stars descends.
        assert_eq!(
            execute(&c, &Query::parse("sort:stars limit:1"))
                .first()
                .unwrap()
                .repo
                .full_name(),
            "tauri-apps/tauri"
        );
        assert_eq!(
            execute(&c, &Query::parse("sort:-stars limit:1"))
                .first()
                .unwrap()
                .repo
                .full_name(),
            "golang/go"
        );
    }
}
