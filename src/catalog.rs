//! The catalogue: repos as library volumes, shelves as collections,
//! faceted filtering over language / topic / license / stars / installed.

use crate::json::Json;
use crate::repo_ref::RepoRef;
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq)]
pub struct RepoEntry {
    pub repo: RepoRef,
    pub description: String,
    pub stars: u64,
    pub language: String,
    pub license: String,
    pub topics: Vec<String>,
    pub latest_tag: String,
    pub installed_tag: String,
    pub auto_download: bool,
    pub asset_patterns: Vec<String>,
    pub last_metadata_fetch: u64,
    pub last_release_check: u64,
    pub etag: String,
    /// When the volume entered the catalogue (unix seconds; 0 = unknown,
    /// e.g. entries written before this field existed).
    pub added_unix: u64,
}

impl RepoEntry {
    pub fn new(repo: RepoRef) -> RepoEntry {
        RepoEntry {
            repo,
            description: String::new(),
            stars: 0,
            language: String::new(),
            license: String::new(),
            topics: Vec::new(),
            latest_tag: String::new(),
            installed_tag: String::new(),
            auto_download: false,
            asset_patterns: Vec::new(),
            last_metadata_fetch: 0,
            last_release_check: 0,
            etag: String::new(),
            added_unix: 0,
        }
    }

    pub fn is_installed(&self) -> bool {
        !self.installed_tag.is_empty()
    }

    pub fn update_available(&self) -> bool {
        self.is_installed() && !self.latest_tag.is_empty() && self.latest_tag != self.installed_tag
    }

    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("repo".into(), Json::Str(self.repo.full_name())),
            ("description".into(), Json::Str(self.description.clone())),
            ("stars".into(), Json::Num(self.stars as f64)),
            ("language".into(), Json::Str(self.language.clone())),
            ("license".into(), Json::Str(self.license.clone())),
            (
                "topics".into(),
                Json::Arr(self.topics.iter().map(|t| Json::Str(t.clone())).collect()),
            ),
            ("latest_tag".into(), Json::Str(self.latest_tag.clone())),
            (
                "installed_tag".into(),
                Json::Str(self.installed_tag.clone()),
            ),
            ("auto_download".into(), Json::Bool(self.auto_download)),
            (
                "asset_patterns".into(),
                Json::Arr(
                    self.asset_patterns
                        .iter()
                        .map(|t| Json::Str(t.clone()))
                        .collect(),
                ),
            ),
            (
                "last_metadata_fetch".into(),
                Json::Num(self.last_metadata_fetch as f64),
            ),
            (
                "last_release_check".into(),
                Json::Num(self.last_release_check as f64),
            ),
            ("etag".into(), Json::Str(self.etag.clone())),
            ("added_unix".into(), Json::Num(self.added_unix as f64)),
        ])
    }

    fn from_json(v: &Json) -> Option<RepoEntry> {
        let repo = RepoRef::parse(v.get("repo")?.as_str()?).ok()?;
        let mut e = RepoEntry::new(repo);
        e.description = v
            .get("description")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into();
        e.stars = v.get("stars").and_then(Json::as_f64).unwrap_or(0.0) as u64;
        e.language = v
            .get("language")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into();
        e.license = v.get("license").and_then(Json::as_str).unwrap_or("").into();
        if let Some(Json::Arr(items)) = v.get("topics") {
            e.topics = items
                .iter()
                .filter_map(Json::as_str)
                .map(str::to_string)
                .collect();
        }
        e.latest_tag = v
            .get("latest_tag")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into();
        e.installed_tag = v
            .get("installed_tag")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into();
        e.auto_download = v
            .get("auto_download")
            .and_then(Json::as_bool)
            .unwrap_or(false);
        if let Some(Json::Arr(items)) = v.get("asset_patterns") {
            e.asset_patterns = items
                .iter()
                .filter_map(Json::as_str)
                .map(str::to_string)
                .collect();
        }
        e.last_metadata_fetch = v
            .get("last_metadata_fetch")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u64;
        e.last_release_check = v
            .get("last_release_check")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u64;
        e.etag = v.get("etag").and_then(Json::as_str).unwrap_or("").into();
        e.added_unix = v.get("added_unix").and_then(Json::as_f64).unwrap_or(0.0) as u64;
        Some(e)
    }
}

/// Outcome of a bulk import — the counts the Add Volumes preview shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub duplicates: Vec<String>,
    pub invalid: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub entries: Vec<RepoEntry>,
    /// Shelf name -> full repo names ("owner/repo"), in shelf order.
    pub shelves: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub text: String,
    pub language: String,
    pub topic: String,
    pub license: String,
    pub installed_only: bool,
    pub with_releases_only: bool,
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Catalog {
    /// Add a repo; returns false when it was already catalogued.
    pub fn add(&mut self, repo: RepoRef) -> bool {
        if self.get(&repo).is_some() {
            return false;
        }
        let mut entry = RepoEntry::new(repo);
        entry.added_unix = now_unix();
        self.entries.push(entry);
        true
    }

    /// Bulk import: one repo per line (`owner/repo` or a github.com URL).
    /// Blank lines and `#` comments are skipped. Returns what was added,
    /// what was already present, and what did not parse — the preview the
    /// Add Volumes dialog shows before anything is committed.
    pub fn import_list(&mut self, text: &str) -> ImportReport {
        let mut report = ImportReport::default();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match RepoRef::parse(line) {
                Ok(repo) => {
                    if self.add(repo.clone()) {
                        report.added.push(repo.full_name());
                    } else {
                        report.duplicates.push(repo.full_name());
                    }
                }
                Err(e) => report.invalid.push((line.to_string(), e.0)),
            }
        }
        report
    }

    pub fn get(&self, repo: &RepoRef) -> Option<&RepoEntry> {
        self.entries.iter().find(|e| &e.repo == repo)
    }

    pub fn get_mut(&mut self, repo: &RepoRef) -> Option<&mut RepoEntry> {
        self.entries.iter_mut().find(|e| &e.repo == repo)
    }

    pub fn remove(&mut self, repo: &RepoRef) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| &e.repo != repo);
        for members in self.shelves.values_mut() {
            members.retain(|m| m != &repo.full_name());
        }
        self.entries.len() != before
    }

    pub fn add_to_shelf(&mut self, shelf: &str, repo: &RepoRef) -> bool {
        if self.get(repo).is_none() {
            return false;
        }
        let members = self.shelves.entry(shelf.to_string()).or_default();
        if members.contains(&repo.full_name()) {
            return false;
        }
        members.push(repo.full_name());
        true
    }

    pub fn remove_from_shelf(&mut self, shelf: &str, repo: &RepoRef) -> bool {
        let name = repo.full_name();
        if let Some((_, members)) = self.shelves.iter_mut().find(|(n, _)| n.as_str() == shelf) {
            let before = members.len();
            members.retain(|m| m != &name);
            return members.len() != before;
        }
        false
    }

    pub fn shelf_members(&self, shelf: &str) -> Vec<&RepoEntry> {
        match self.shelves.get(shelf) {
            Some(names) => names
                .iter()
                .filter_map(|n| RepoRef::parse(n).ok())
                .filter_map(|r| self.get(&r))
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn filter(&self, f: &Filter) -> Vec<&RepoEntry> {
        self.entries
            .iter()
            .filter(|e| {
                if f.installed_only && !e.is_installed() {
                    return false;
                }
                if f.with_releases_only && e.latest_tag.is_empty() {
                    return false;
                }
                if !f.language.is_empty() && !e.language.eq_ignore_ascii_case(&f.language) {
                    return false;
                }
                if !f.license.is_empty() && !e.license.eq_ignore_ascii_case(&f.license) {
                    return false;
                }
                if !f.topic.is_empty() && !e.topics.iter().any(|t| t.eq_ignore_ascii_case(&f.topic))
                {
                    return false;
                }
                if !f.text.is_empty() {
                    let q = f.text.to_lowercase();
                    let hay = format!(
                        "{} {} {} {}",
                        e.repo.full_name(),
                        e.description,
                        e.language,
                        e.topics.join(" ")
                    )
                    .to_lowercase();
                    if !hay.contains(&q) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Facet counts for the filter panel: (language, count), sorted by count.
    pub fn language_facets(&self) -> Vec<(String, usize)> {
        count_facets(self.entries.iter().filter_map(|e| {
            if e.language.is_empty() {
                None
            } else {
                Some(e.language.clone())
            }
        }))
    }

    pub fn topic_facets(&self) -> Vec<(String, usize)> {
        count_facets(self.entries.iter().flat_map(|e| e.topics.clone()))
    }

    pub fn license_facets(&self) -> Vec<(String, usize)> {
        count_facets(self.entries.iter().filter_map(|e| {
            if e.license.is_empty() {
                None
            } else {
                Some(e.license.clone())
            }
        }))
    }

    pub fn owner_facets(&self) -> Vec<(String, usize)> {
        count_facets(self.entries.iter().map(|e| e.repo.owner.clone()))
    }

    pub fn updates_available(&self) -> Vec<&RepoEntry> {
        self.entries
            .iter()
            .filter(|e| e.update_available())
            .collect()
    }

    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("schema_version".into(), Json::Num(SCHEMA_VERSION as f64)),
            (
                "repos".into(),
                Json::Arr(self.entries.iter().map(RepoEntry::to_json).collect()),
            ),
            (
                "shelves".into(),
                Json::Obj(
                    self.shelves
                        .iter()
                        .map(|(name, members)| {
                            (
                                name.clone(),
                                Json::Arr(members.iter().map(|m| Json::Str(m.clone())).collect()),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }

    pub fn from_json(v: &Json) -> Result<Catalog, String> {
        let version = v
            .get("schema_version")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u32;
        if version != SCHEMA_VERSION {
            return Err(format!(
                "catalogue schema_version {version} is not supported (expected {SCHEMA_VERSION})"
            ));
        }
        let mut cat = Catalog::default();
        if let Some(Json::Arr(items)) = v.get("repos") {
            for item in items {
                if let Some(e) = RepoEntry::from_json(item) {
                    cat.entries.push(e);
                }
            }
        }
        if let Some(Json::Obj(shelves)) = v.get("shelves") {
            for (name, members) in shelves {
                if let Json::Arr(items) = members {
                    cat.shelves.insert(
                        name.clone(),
                        items
                            .iter()
                            .filter_map(Json::as_str)
                            .map(str::to_string)
                            .collect(),
                    );
                }
            }
        }
        Ok(cat)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::persist::save_atomic(path, &self.to_json().to_string_pretty())
    }

    pub fn load(path: &std::path::Path) -> Result<Catalog, String> {
        crate::persist::load_recovering(path, |text| {
            let v = crate::json::parse(text).map_err(|e| format!("parse catalogue: {e}"))?;
            Catalog::from_json(&v)
        })
    }
}

fn count_facets(values: impl Iterator<Item = String>) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        let mut c = Catalog::default();
        c.add(RepoRef::parse("rust-lang/rust").unwrap());
        c.add(RepoRef::parse("tauri-apps/tauri").unwrap());
        let e = c
            .get_mut(&RepoRef::parse("rust-lang/rust").unwrap())
            .unwrap();
        e.language = "Rust".into();
        e.topics = vec!["compiler".into()];
        e.latest_tag = "1.99".into();
        e.installed_tag = "1.98".into();
        c
    }

    #[test]
    fn add_dedups_and_shelves_hold_members() {
        let mut c = cat();
        assert!(!c.add(RepoRef::parse("rust-lang/rust").unwrap()));
        let r = RepoRef::parse("tauri-apps/tauri").unwrap();
        assert!(c.add_to_shelf("Dev Tools", &r));
        assert!(!c.add_to_shelf("Dev Tools", &r));
        assert_eq!(c.shelf_members("Dev Tools").len(), 1);
        assert!(c.remove(&r));
        assert!(c.shelf_members("Dev Tools").is_empty());
    }

    #[test]
    fn filters_and_facets_work() {
        let c = cat();
        assert_eq!(
            c.filter(&Filter {
                text: "rust".into(),
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(
            c.filter(&Filter {
                language: "rust".into(),
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(
            c.filter(&Filter {
                installed_only: true,
                ..Default::default()
            })
            .len(),
            1
        );
        assert_eq!(c.language_facets(), vec![("Rust".to_string(), 1)]);
        assert_eq!(c.updates_available().len(), 1);
    }

    #[test]
    fn owner_and_license_facets_and_added_stamp() {
        let mut c = cat();
        let e = c
            .get_mut(&RepoRef::parse("rust-lang/rust").unwrap())
            .unwrap();
        e.license = "MIT".into();
        e.added_unix = 1234;
        assert_eq!(c.license_facets(), vec![("MIT".to_string(), 1)]);
        assert_eq!(c.owner_facets().len(), 2);
        // added_unix survives the JSON round trip; old files default to 0.
        let text = c.to_json().to_string_pretty();
        let back = Catalog::from_json(&crate::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back.entries[0].added_unix, 1234);
        let old =
            crate::json::parse("{\"schema_version\":1,\"repos\":[{\"repo\":\"a/b\"}]}").unwrap();
        let back_old = Catalog::from_json(&old).unwrap();
        assert_eq!(back_old.entries[0].added_unix, 0);
    }

    #[test]
    fn json_round_trip_and_schema_guard() {
        let c = cat();
        let text = c.to_json().to_string_pretty();
        let back = Catalog::from_json(&crate::json::parse(&text).unwrap()).unwrap();
        assert_eq!(back.entries.len(), 2);
        assert_eq!(back.entries[0].repo.full_name(), "rust-lang/rust");
        let bad = crate::json::parse("{\"schema_version\": 99, \"repos\": []}").unwrap();
        assert!(Catalog::from_json(&bad).is_err());
    }

    #[test]
    fn bulk_import_reports_added_duplicates_and_invalid() {
        let mut c = Catalog::default();
        c.add(RepoRef::parse("sharkdp/bat").unwrap());
        let text = "# my list\nsharkdp/bat\nBurntSushi/ripgrep\n\nhttps://github.com/casey/just\nnot a repo\n";
        let report = c.import_list(text);
        assert_eq!(report.added, vec!["BurntSushi/ripgrep", "casey/just"]);
        assert_eq!(report.duplicates, vec!["sharkdp/bat"]);
        // GitHub identity is case-insensitive: a case variant is a duplicate.
        let report2 = c.import_list("SHARKDP/BAT\n");
        assert_eq!(report2.duplicates.len(), 1);
        assert!(report2.added.is_empty());
        assert_eq!(report.invalid.len(), 1);
        assert_eq!(c.entries.len(), 3);
    }
}
