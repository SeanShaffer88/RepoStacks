//! Presentation: switchable layouts, themes, density, grouping, and
//! column choice — all as data, so the CLI renders every layout today and
//! the window renders the same layouts later from the same settings.
//!
//! Settings live in their own file (`data/settings.json`, schema_version 1,
//! atomic writes) next to the catalogue, and include saved searches.

use crate::catalog::{Catalog, RepoEntry};
use crate::json::Json;
use std::collections::BTreeMap;

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// The approved default: spreadsheet-style table on a dark ground.
    Ledger,
    /// Book-cover style cards in a grid.
    Cards,
    /// One line per volume, densest reading view.
    Compact,
    /// Shelf zones on a canvas, volumes as field cards (diagram view).
    Diagram,
}

impl Layout {
    pub fn name(&self) -> &'static str {
        match self {
            Layout::Ledger => "ledger",
            Layout::Cards => "cards",
            Layout::Compact => "compact",
            Layout::Diagram => "diagram",
        }
    }
    pub fn from_name(s: &str) -> Option<Layout> {
        match s.to_lowercase().as_str() {
            "ledger" | "table" | "spreadsheet" => Some(Layout::Ledger),
            "cards" | "grid" | "covers" => Some(Layout::Cards),
            "compact" | "list" | "lines" => Some(Layout::Compact),
            "diagram" | "canvas" | "shelves" => Some(Layout::Diagram),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    DarkLedger,
    PaperLedger,
    CleanSheet,
    Terminal,
}

impl Theme {
    pub fn name(&self) -> &'static str {
        match self {
            Theme::DarkLedger => "dark-ledger",
            Theme::PaperLedger => "paper-ledger",
            Theme::CleanSheet => "clean-sheet",
            Theme::Terminal => "terminal",
        }
    }
    pub fn from_name(s: &str) -> Option<Theme> {
        match s.to_lowercase().as_str() {
            "dark-ledger" | "dark" => Some(Theme::DarkLedger),
            "paper-ledger" | "paper" => Some(Theme::PaperLedger),
            "clean-sheet" | "clean" | "light" => Some(Theme::CleanSheet),
            "terminal" | "mono" => Some(Theme::Terminal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Comfortable,
    Compact,
}

impl Density {
    pub fn from_name(s: &str) -> Option<Density> {
        match s.to_lowercase().as_str() {
            "comfortable" | "roomy" => Some(Density::Comfortable),
            "compact" | "dense" => Some(Density::Compact),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Density::Comfortable => "comfortable",
            Density::Compact => "compact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    None,
    Shelf,
    Language,
    License,
    Status,
}

impl GroupBy {
    pub fn from_name(s: &str) -> Option<GroupBy> {
        match s.to_lowercase().as_str() {
            "none" | "off" => Some(GroupBy::None),
            "shelf" => Some(GroupBy::Shelf),
            "language" | "lang" => Some(GroupBy::Language),
            "license" => Some(GroupBy::License),
            "status" => Some(GroupBy::Status),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            GroupBy::None => "none",
            GroupBy::Shelf => "shelf",
            GroupBy::Language => "language",
            GroupBy::License => "license",
            GroupBy::Status => "status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Title,
    Owner,
    Language,
    License,
    Stars,
    LatestRelease,
    Installed,
    Status,
    Topics,
    Shelf,
}

impl Column {
    pub fn header(&self) -> &'static str {
        match self {
            Column::Title => "Title",
            Column::Owner => "Owner",
            Column::Language => "Language",
            Column::License => "License",
            Column::Stars => "Stars",
            Column::LatestRelease => "Latest Release",
            Column::Installed => "Installed",
            Column::Status => "Status",
            Column::Topics => "Topics",
            Column::Shelf => "Shelf",
        }
    }
    pub fn from_name(s: &str) -> Option<Column> {
        match s.to_lowercase().as_str() {
            "title" | "name" => Some(Column::Title),
            "owner" => Some(Column::Owner),
            "language" | "lang" => Some(Column::Language),
            "license" => Some(Column::License),
            "stars" => Some(Column::Stars),
            "release" | "latest" | "latestrelease" => Some(Column::LatestRelease),
            "installed" => Some(Column::Installed),
            "status" => Some(Column::Status),
            "topics" => Some(Column::Topics),
            "shelf" => Some(Column::Shelf),
            _ => None,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            Column::Title => "title",
            Column::Owner => "owner",
            Column::Language => "language",
            Column::License => "license",
            Column::Stars => "stars",
            Column::LatestRelease => "release",
            Column::Installed => "installed",
            Column::Status => "status",
            Column::Topics => "topics",
            Column::Shelf => "shelf",
        }
    }
    pub fn value(&self, e: &RepoEntry, catalog: &Catalog) -> String {
        match self {
            Column::Title => e.repo.full_name(),
            Column::Owner => e.repo.owner.clone(),
            Column::Language => dash(&e.language),
            Column::License => dash(&e.license),
            Column::Stars => format!("{}", e.stars),
            Column::LatestRelease => dash(&e.latest_tag),
            Column::Installed => dash(&e.installed_tag),
            Column::Status => status_of(e).to_string(),
            Column::Topics => {
                if e.topics.is_empty() {
                    "—".into()
                } else {
                    e.topics.join(", ")
                }
            }
            Column::Shelf => {
                let names: Vec<&str> = catalog
                    .shelves
                    .iter()
                    .filter(|(_, m)| m.contains(&e.repo.full_name()))
                    .map(|(n, _)| n.as_str())
                    .collect();
                if names.is_empty() {
                    "—".into()
                } else {
                    names.join(", ")
                }
            }
        }
    }
}

pub fn status_of(e: &RepoEntry) -> &'static str {
    if e.update_available() {
        "Update available"
    } else if e.is_installed() {
        "Installed"
    } else if !e.latest_tag.is_empty() {
        "Not installed"
    } else {
        "No releases"
    }
}

fn dash(s: &str) -> String {
    if s.is_empty() {
        "—".into()
    } else {
        s.to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewConfig {
    pub layout: Layout,
    pub theme: Theme,
    pub density: Density,
    pub group_by: GroupBy,
    pub columns: Vec<Column>,
    pub show_facets: bool,
}

impl Default for ViewConfig {
    fn default() -> ViewConfig {
        ViewConfig {
            layout: Layout::Ledger,
            theme: Theme::DarkLedger,
            density: Density::Comfortable,
            group_by: GroupBy::None,
            columns: vec![
                Column::Title,
                Column::Owner,
                Column::Language,
                Column::License,
                Column::Stars,
                Column::LatestRelease,
                Column::Installed,
                Column::Status,
            ],
            show_facets: true,
        }
    }
}

impl ViewConfig {
    fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("layout".into(), Json::Str(self.layout.name().into())),
            ("theme".into(), Json::Str(self.theme.name().into())),
            ("density".into(), Json::Str(self.density.name().into())),
            ("group_by".into(), Json::Str(self.group_by.name().into())),
            (
                "columns".into(),
                Json::Arr(
                    self.columns
                        .iter()
                        .map(|c| Json::Str(c.name().into()))
                        .collect(),
                ),
            ),
            ("show_facets".into(), Json::Bool(self.show_facets)),
        ])
    }
    fn from_json(v: &Json) -> ViewConfig {
        let mut out = ViewConfig::default();
        if let Some(s) = v.get("layout").and_then(Json::as_str) {
            if let Some(l) = Layout::from_name(s) {
                out.layout = l;
            }
        }
        if let Some(s) = v.get("theme").and_then(Json::as_str) {
            if let Some(t) = Theme::from_name(s) {
                out.theme = t;
            }
        }
        if let Some(s) = v.get("density").and_then(Json::as_str) {
            if let Some(d) = Density::from_name(s) {
                out.density = d;
            }
        }
        if let Some(s) = v.get("group_by").and_then(Json::as_str) {
            if let Some(g) = GroupBy::from_name(s) {
                out.group_by = g;
            }
        }
        if let Some(Json::Arr(items)) = v.get("columns") {
            let cols: Vec<Column> = items
                .iter()
                .filter_map(Json::as_str)
                .filter_map(Column::from_name)
                .collect();
            if !cols.is_empty() {
                out.columns = cols;
            }
        }
        if let Some(b) = v.get("show_facets").and_then(Json::as_bool) {
            out.show_facets = b;
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub view: ViewConfig,
    pub saved_searches: BTreeMap<String, String>,
    /// Where downloads land (a folder per volume, a subfolder per tag).
    pub download_dir: String,
    /// How many downloads run at once (the Librarian's setting).
    pub concurrency: u32,
    /// Librarian schedule: seconds between Atom release checks.
    pub atom_poll_secs: u64,
    /// Librarian schedule: seconds between metadata refreshes.
    pub metadata_poll_secs: u64,
    /// Run a Librarian pass (checks + metadata + standing orders) once
    /// when the window opens. Off by default — opening the app should
    /// not spend quota unless the user asked for it.
    pub check_on_start: bool,
    /// Optional GitHub token, DPAPI-protected and base64-encoded by the
    /// window. Never a plain token; the CLI uses REPOSTACKS_TOKEN instead.
    pub token_protected: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            view: ViewConfig::default(),
            saved_searches: BTreeMap::new(),
            download_dir: "downloads".to_string(),
            concurrency: 3,
            atom_poll_secs: 21_600,
            metadata_poll_secs: 86_400,
            check_on_start: false,
            token_protected: String::new(),
        }
    }
}

impl Settings {
    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            (
                "schema_version".into(),
                Json::Num(SETTINGS_SCHEMA_VERSION as f64),
            ),
            ("view".into(), self.view.to_json()),
            (
                "saved_searches".into(),
                Json::Obj(
                    self.saved_searches
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::Str(v.clone())))
                        .collect(),
                ),
            ),
            ("download_dir".into(), Json::Str(self.download_dir.clone())),
            ("concurrency".into(), Json::Num(self.concurrency as f64)),
            (
                "atom_poll_secs".into(),
                Json::Num(self.atom_poll_secs as f64),
            ),
            (
                "metadata_poll_secs".into(),
                Json::Num(self.metadata_poll_secs as f64),
            ),
            ("check_on_start".into(), Json::Bool(self.check_on_start)),
            (
                "token_protected".into(),
                Json::Str(self.token_protected.clone()),
            ),
        ])
    }
    pub fn from_json(v: &Json) -> Result<Settings, String> {
        let version = v
            .get("schema_version")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u32;
        if version != SETTINGS_SCHEMA_VERSION {
            return Err(format!("settings schema_version {version} is not supported (expected {SETTINGS_SCHEMA_VERSION})"));
        }
        let mut s = Settings::default();
        if let Some(view) = v.get("view") {
            s.view = ViewConfig::from_json(view);
        }
        if let Some(Json::Obj(map)) = v.get("saved_searches") {
            for (k, val) in map {
                if let Some(q) = val.as_str() {
                    s.saved_searches.insert(k.clone(), q.to_string());
                }
            }
        }
        if let Some(d) = v.get("download_dir").and_then(Json::as_str) {
            if !d.is_empty() {
                s.download_dir = d.to_string();
            }
        }
        if let Some(n) = v.get("concurrency").and_then(Json::as_f64) {
            if n >= 1.0 {
                s.concurrency = n as u32;
            }
        }
        if let Some(n) = v.get("atom_poll_secs").and_then(Json::as_f64) {
            if n >= 60.0 {
                s.atom_poll_secs = n as u64;
            }
        }
        if let Some(n) = v.get("metadata_poll_secs").and_then(Json::as_f64) {
            if n >= 60.0 {
                s.metadata_poll_secs = n as u64;
            }
        }
        if let Some(tok) = v.get("token_protected").and_then(Json::as_str) {
            s.token_protected = tok.to_string();
        }
        if let Some(b) = v.get("check_on_start").and_then(Json::as_bool) {
            s.check_on_start = b;
        }
        Ok(s)
    }
    /// Check settings a user is about to save (as opposed to settings
    /// being loaded, which stay tolerant of out-of-range values). Errors
    /// are plain-language and name the field and the allowed range.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.concurrency) {
            return Err(format!(
                "download concurrency must be between 1 and 16 (got {})",
                self.concurrency
            ));
        }
        if !(60..=604_800).contains(&self.atom_poll_secs) {
            return Err(format!(
                "release-check interval must be between 1 minute and 7 days (got {}s)",
                self.atom_poll_secs
            ));
        }
        if !(60..=604_800).contains(&self.metadata_poll_secs) {
            return Err(format!(
                "metadata-refresh interval must be between 1 minute and 7 days (got {}s)",
                self.metadata_poll_secs
            ));
        }
        if self.download_dir.trim().is_empty() {
            return Err("the downloads folder may not be empty".into());
        }
        Ok(())
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::persist::save_atomic(path, &self.to_json().to_string_pretty())
    }
    pub fn load(path: &std::path::Path) -> Result<Settings, String> {
        crate::persist::load_recovering(path, |text| {
            let v = crate::json::parse(text).map_err(|e| format!("parse settings: {e}"))?;
            Settings::from_json(&v)
        })
    }
}

/// Render entries in the configured layout. The CLI uses this text form;
/// the window will render the same layouts graphically from the same
/// `ViewConfig`, so what a user saves here is what the window opens with.
pub fn render(entries: &[&RepoEntry], catalog: &Catalog, view: &ViewConfig) -> String {
    if entries.is_empty() {
        return "No volumes match.\n".to_string();
    }
    let groups = group_entries(entries, catalog, view.group_by);
    let mut out = String::new();
    for (label, members) in &groups {
        if view.group_by != GroupBy::None {
            out.push_str(&format!("== {label} ==\n"));
        }
        match view.layout {
            Layout::Ledger => render_ledger(members, catalog, view, &mut out),
            Layout::Compact => {
                for e in members {
                    out.push_str(&format!(
                        "{}  ·  {}  ·  {}  ·  ★{}  ·  {}  ·  {}\n",
                        e.repo.full_name(),
                        dash(&e.language),
                        dash(&e.license),
                        e.stars,
                        dash(&e.latest_tag),
                        status_of(e)
                    ));
                }
            }
            Layout::Cards => {
                for e in members {
                    out.push_str(&format!(
                        "┌ {} ─────────────\n│ {} · ★{} · {}\n│ release {} · installed {} · {}\n┘\n",
                        e.repo.full_name(),
                        dash(&e.language),
                        e.stars,
                        dash(&e.license),
                        dash(&e.latest_tag),
                        dash(&e.installed_tag),
                        status_of(e)
                    ));
                }
            }
            Layout::Diagram => {
                for e in members {
                    out.push_str(&format!(
                        "[{}]\n  language {} | license {} | stars {}\n  release {} | installed {} | {}\n",
                        e.repo.full_name(),
                        dash(&e.language),
                        dash(&e.license),
                        e.stars,
                        dash(&e.latest_tag),
                        dash(&e.installed_tag),
                        status_of(e)
                    ));
                }
            }
        }
    }
    out
}

fn group_entries<'a>(
    entries: &[&'a RepoEntry],
    catalog: &Catalog,
    group_by: GroupBy,
) -> Vec<(String, Vec<&'a RepoEntry>)> {
    if group_by == GroupBy::None {
        return vec![(String::new(), entries.to_vec())];
    }
    let mut groups: BTreeMap<String, Vec<&RepoEntry>> = BTreeMap::new();
    for e in entries {
        let key = match group_by {
            GroupBy::Shelf => Column::Shelf.value(e, catalog),
            GroupBy::Language => dash(&e.language),
            GroupBy::License => dash(&e.license),
            GroupBy::Status => status_of(e).to_string(),
            GroupBy::None => unreachable!(),
        };
        groups.entry(key).or_default().push(e);
    }
    groups.into_iter().collect()
}

fn render_ledger(members: &[&RepoEntry], catalog: &Catalog, view: &ViewConfig, out: &mut String) {
    let cols = &view.columns;
    let mut widths: Vec<usize> = cols.iter().map(|c| c.header().len()).collect();
    let rows: Vec<Vec<String>> = members
        .iter()
        .map(|e| {
            cols.iter()
                .enumerate()
                .map(|(i, c)| {
                    let v = c.value(e, catalog);
                    widths[i] = widths[i].max(v.chars().count());
                    v
                })
                .collect()
        })
        .collect();
    let header: Vec<String> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{:width$}", c.header(), width = widths[i]))
        .collect();
    out.push_str(&header.join(" | "));
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    out.push('\n');
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{:width$}", v, width = widths[i]))
            .collect();
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_ref::RepoRef;

    fn cat() -> Catalog {
        let mut c = Catalog::default();
        c.add(RepoRef::parse("rust-lang/rust").unwrap());
        let e = c
            .get_mut(&RepoRef::parse("rust-lang/rust").unwrap())
            .unwrap();
        e.language = "Rust".into();
        e.license = "MIT".into();
        e.stars = 10;
        e.latest_tag = "v1".into();
        c
    }

    #[test]
    fn settings_validate_bounds() {
        assert!(Settings::default().validate().is_ok());
        let mut s = Settings::default();
        s.concurrency = 0;
        assert!(s.validate().is_err());
        s.concurrency = 17;
        assert!(s.validate().is_err());
        s.concurrency = 3;
        s.atom_poll_secs = 59;
        assert!(s.validate().is_err());
        s.atom_poll_secs = 21_600;
        s.download_dir = "  ".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn settings_round_trip_and_defaults_are_dark_ledger() {
        let mut s = Settings::default();
        assert_eq!(s.view.layout, Layout::Ledger);
        assert_eq!(s.view.theme, Theme::DarkLedger);
        s.view.layout = Layout::Cards;
        s.view.columns = vec![Column::Title, Column::Status];
        s.saved_searches.insert("rusty".into(), "lang:rust".into());
        let back =
            Settings::from_json(&crate::json::parse(&s.to_json().to_string_pretty()).unwrap())
                .unwrap();
        assert_eq!(back, s);
        let bad = crate::json::parse("{\"schema_version\": 7}").unwrap();
        assert!(Settings::from_json(&bad).is_err());
    }

    #[test]
    fn renders_each_layout_and_groups() {
        let c = cat();
        let entries: Vec<&RepoEntry> = c.entries.iter().collect();
        for layout in [
            Layout::Ledger,
            Layout::Cards,
            Layout::Compact,
            Layout::Diagram,
        ] {
            let mut v = ViewConfig::default();
            v.layout = layout;
            let text = render(&entries, &c, &v);
            assert!(
                text.contains("rust-lang/rust"),
                "layout {} lost the entry",
                layout.name()
            );
        }
        let mut v = ViewConfig::default();
        v.group_by = GroupBy::Language;
        assert!(render(&entries, &c, &v).contains("== Rust =="));
        let ledger = render(&entries, &c, &ViewConfig::default());
        assert!(ledger.contains("Latest Release"));
    }
}
