//! Persistent accounting for the REST calls RepoStacks itself makes.
//!
//! GitHub does not give this app a live "remaining calls" feed, so the
//! meter is deliberately honest about what it is: a local ledger of the
//! calls RepoStacks counted under its cache policy. Atom release checks
//! are not REST calls and are never recorded. Current-hour and current
//! UTC-day totals roll over by clock bucket; lifetime totals remain.

use crate::cache_policy::Auth;
use crate::json::Json;
use std::collections::BTreeMap;
use std::path::Path;

pub const USAGE_SCHEMA_VERSION: u32 = 1;
const HOUR_SECS: u64 = 3_600;
const DAY_SECS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UsageKind {
    Metadata,
    Assets,
    CheckFallback,
    Imports,
}

impl UsageKind {
    pub fn name(&self) -> &'static str {
        match self {
            UsageKind::Metadata => "metadata",
            UsageKind::Assets => "assets",
            UsageKind::CheckFallback => "check_fallback",
            UsageKind::Imports => "imports",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            UsageKind::Metadata => "Metadata refresh",
            UsageKind::Assets => "Release asset lists",
            UsageKind::CheckFallback => "Release-check fallback",
            UsageKind::Imports => "Account imports",
        }
    }

    pub fn from_name(name: &str) -> Option<UsageKind> {
        match name {
            "metadata" => Some(UsageKind::Metadata),
            "assets" => Some(UsageKind::Assets),
            "check_fallback" => Some(UsageKind::CheckFallback),
            "imports" => Some(UsageKind::Imports),
            _ => None,
        }
    }

    pub fn all() -> [UsageKind; 4] {
        [
            UsageKind::Metadata,
            UsageKind::Assets,
            UsageKind::CheckFallback,
            UsageKind::Imports,
        ]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSnapshot {
    pub auth: Auth,
    pub hourly_limit: u32,
    pub hour_start_unix: u64,
    pub hour_reset_unix: u64,
    pub hour_spent: u32,
    pub day_start_unix: u64,
    pub day_spent: u32,
    pub lifetime_spent: u64,
    pub last_spent_unix: u64,
    pub hour_by_kind: BTreeMap<UsageKind, u32>,
    pub day_by_kind: BTreeMap<UsageKind, u32>,
    pub lifetime_by_kind: BTreeMap<UsageKind, u64>,
}

impl UsageSnapshot {
    pub fn hour_percent(&self) -> f64 {
        if self.hourly_limit == 0 {
            0.0
        } else {
            (self.hour_spent as f64 * 100.0 / self.hourly_limit as f64).min(100.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageLedger {
    hour_start_unix: u64,
    day_start_unix: u64,
    hour_spent: u32,
    day_spent: u32,
    lifetime_spent: u64,
    last_spent_unix: u64,
    hour_by_kind: BTreeMap<UsageKind, u32>,
    day_by_kind: BTreeMap<UsageKind, u32>,
    lifetime_by_kind: BTreeMap<UsageKind, u64>,
}

impl Default for UsageLedger {
    fn default() -> UsageLedger {
        UsageLedger {
            hour_start_unix: 0,
            day_start_unix: 0,
            hour_spent: 0,
            day_spent: 0,
            lifetime_spent: 0,
            last_spent_unix: 0,
            hour_by_kind: BTreeMap::new(),
            day_by_kind: BTreeMap::new(),
            lifetime_by_kind: BTreeMap::new(),
        }
    }
}

fn hour_start(now_unix: u64) -> u64 {
    now_unix - (now_unix % HOUR_SECS)
}

fn day_start(now_unix: u64) -> u64 {
    now_unix - (now_unix % DAY_SECS)
}

fn kind_map_u32_json(map: &BTreeMap<UsageKind, u32>) -> Json {
    Json::Obj(
        map.iter()
            .filter(|(_, calls)| **calls > 0)
            .map(|(kind, calls)| (kind.name().into(), Json::Num(*calls as f64)))
            .collect(),
    )
}

fn kind_map_u64_json(map: &BTreeMap<UsageKind, u64>) -> Json {
    Json::Obj(
        map.iter()
            .filter(|(_, calls)| **calls > 0)
            .map(|(kind, calls)| (kind.name().into(), Json::Num(*calls as f64)))
            .collect(),
    )
}

fn read_kind_map_u32(v: Option<&Json>) -> BTreeMap<UsageKind, u32> {
    let mut out = BTreeMap::new();
    if let Some(Json::Obj(pairs)) = v {
        for (name, value) in pairs {
            if let (Some(kind), Some(calls)) = (UsageKind::from_name(name), value.as_f64()) {
                if calls > 0.0 {
                    out.insert(kind, calls as u32);
                }
            }
        }
    }
    out
}

fn read_kind_map_u64(v: Option<&Json>) -> BTreeMap<UsageKind, u64> {
    let mut out = BTreeMap::new();
    if let Some(Json::Obj(pairs)) = v {
        for (name, value) in pairs {
            if let (Some(kind), Some(calls)) = (UsageKind::from_name(name), value.as_f64()) {
                if calls > 0.0 {
                    out.insert(kind, calls as u64);
                }
            }
        }
    }
    out
}

impl UsageLedger {
    pub fn record(&mut self, kind: UsageKind, calls: u32, now_unix: u64) {
        if calls == 0 {
            return;
        }
        self.roll_to(now_unix);
        self.hour_spent = self.hour_spent.saturating_add(calls);
        self.day_spent = self.day_spent.saturating_add(calls);
        self.lifetime_spent = self.lifetime_spent.saturating_add(calls as u64);
        *self.hour_by_kind.entry(kind).or_default() += calls;
        *self.day_by_kind.entry(kind).or_default() += calls;
        *self.lifetime_by_kind.entry(kind).or_default() += calls as u64;
        self.last_spent_unix = now_unix;
    }

    fn roll_to(&mut self, now_unix: u64) {
        let current_hour = hour_start(now_unix);
        if self.hour_start_unix == 0 || current_hour > self.hour_start_unix {
            self.hour_start_unix = current_hour;
            self.hour_spent = 0;
            self.hour_by_kind.clear();
        }
        let current_day = day_start(now_unix);
        if self.day_start_unix == 0 || current_day > self.day_start_unix {
            self.day_start_unix = current_day;
            self.day_spent = 0;
            self.day_by_kind.clear();
        }
    }

    pub fn snapshot(&self, auth: Auth, now_unix: u64) -> UsageSnapshot {
        let current_hour = hour_start(now_unix);
        let hour_is_current =
            self.hour_start_unix == current_hour || self.hour_start_unix > current_hour;
        let current_day = day_start(now_unix);
        let day_is_current =
            self.day_start_unix == current_day || self.day_start_unix > current_day;
        UsageSnapshot {
            auth,
            hourly_limit: auth.hourly_rest_limit(),
            hour_start_unix: if hour_is_current {
                self.hour_start_unix
            } else {
                current_hour
            },
            hour_reset_unix: if hour_is_current {
                self.hour_start_unix + HOUR_SECS
            } else {
                current_hour + HOUR_SECS
            },
            hour_spent: if hour_is_current { self.hour_spent } else { 0 },
            day_start_unix: if day_is_current {
                self.day_start_unix
            } else {
                current_day
            },
            day_spent: if day_is_current { self.day_spent } else { 0 },
            lifetime_spent: self.lifetime_spent,
            last_spent_unix: self.last_spent_unix,
            hour_by_kind: if hour_is_current {
                self.hour_by_kind.clone()
            } else {
                BTreeMap::new()
            },
            day_by_kind: if day_is_current {
                self.day_by_kind.clone()
            } else {
                BTreeMap::new()
            },
            lifetime_by_kind: self.lifetime_by_kind.clone(),
        }
    }

    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            (
                "schema_version".into(),
                Json::Num(USAGE_SCHEMA_VERSION as f64),
            ),
            (
                "hour_start_unix".into(),
                Json::Num(self.hour_start_unix as f64),
            ),
            (
                "day_start_unix".into(),
                Json::Num(self.day_start_unix as f64),
            ),
            ("hour_spent".into(), Json::Num(self.hour_spent as f64)),
            ("day_spent".into(), Json::Num(self.day_spent as f64)),
            (
                "lifetime_spent".into(),
                Json::Num(self.lifetime_spent as f64),
            ),
            (
                "last_spent_unix".into(),
                Json::Num(self.last_spent_unix as f64),
            ),
            ("hour_by_kind".into(), kind_map_u32_json(&self.hour_by_kind)),
            ("day_by_kind".into(), kind_map_u32_json(&self.day_by_kind)),
            (
                "lifetime_by_kind".into(),
                kind_map_u64_json(&self.lifetime_by_kind),
            ),
        ])
    }

    pub fn from_json(v: &Json) -> Result<UsageLedger, String> {
        let version = v
            .get("schema_version")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u32;
        if version != USAGE_SCHEMA_VERSION {
            return Err(format!(
                "usage schema_version {version} is not supported (expected {USAGE_SCHEMA_VERSION})"
            ));
        }
        let num = |key: &str| v.get(key).and_then(Json::as_f64).unwrap_or(0.0);
        Ok(UsageLedger {
            hour_start_unix: num("hour_start_unix") as u64,
            day_start_unix: num("day_start_unix") as u64,
            hour_spent: num("hour_spent") as u32,
            day_spent: num("day_spent") as u32,
            lifetime_spent: num("lifetime_spent") as u64,
            last_spent_unix: num("last_spent_unix") as u64,
            hour_by_kind: read_kind_map_u32(v.get("hour_by_kind")),
            day_by_kind: read_kind_map_u32(v.get("day_by_kind")),
            lifetime_by_kind: read_kind_map_u64(v.get("lifetime_by_kind")),
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::persist::save_atomic(path, &self.to_json().to_string_pretty())
    }

    pub fn load(path: &Path) -> Result<UsageLedger, String> {
        crate::persist::load_recovering(path, |text| {
            let v = crate::json::parse(text).map_err(|e| format!("parse usage: {e}"))?;
            UsageLedger::from_json(&v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_by_kind_and_rolls_the_hour_and_day() {
        let mut ledger = UsageLedger::default();
        let start = DAY_SECS + HOUR_SECS + 10;
        ledger.record(UsageKind::Metadata, 2, start);
        ledger.record(UsageKind::Assets, 1, start + 20);
        let snap = ledger.snapshot(Auth::Anonymous, start + 30);
        assert_eq!(snap.hour_spent, 3);
        assert_eq!(snap.day_spent, 3);
        assert_eq!(snap.lifetime_spent, 3);
        assert_eq!(snap.hour_by_kind[&UsageKind::Metadata], 2);

        ledger.record(UsageKind::Imports, 4, start + HOUR_SECS);
        let snap = ledger.snapshot(Auth::Token, start + HOUR_SECS);
        assert_eq!(snap.hour_spent, 4);
        assert_eq!(snap.day_spent, 7);
        assert_eq!(snap.lifetime_spent, 7);
        assert_eq!(snap.hourly_limit, 5_000);

        let tomorrow = ledger.snapshot(Auth::Anonymous, start + DAY_SECS);
        assert_eq!(tomorrow.hour_spent, 0);
        assert_eq!(tomorrow.day_spent, 0);
        assert_eq!(tomorrow.lifetime_spent, 7);
    }

    #[test]
    fn zero_calls_do_not_create_usage() {
        let mut ledger = UsageLedger::default();
        ledger.record(UsageKind::Assets, 0, 123);
        let snap = ledger.snapshot(Auth::Anonymous, 123);
        assert_eq!(snap.lifetime_spent, 0);
        assert_eq!(snap.last_spent_unix, 0);
    }

    #[test]
    fn usage_round_trips_and_rejects_unknown_schema() {
        let mut ledger = UsageLedger::default();
        ledger.record(UsageKind::CheckFallback, 1, DAY_SECS + 5);
        let back = UsageLedger::from_json(
            &crate::json::parse(&ledger.to_json().to_string_pretty()).unwrap(),
        )
        .unwrap();
        assert_eq!(back, ledger);
        let bad = crate::json::parse("{\"schema_version\": 99}").unwrap();
        assert!(UsageLedger::from_json(&bad).is_err());
    }
}
