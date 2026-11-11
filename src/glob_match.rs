//! Small wildcard matcher for release-asset selection rules.
//! Supports `*` (any run) and `?` (one char). Matching is case-insensitive,
//! because Windows asset names vary in case between projects.

pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    match_at(&p, 0, &t, 0)
}

fn eq_ci(a: char, b: char) -> bool {
    a.to_lowercase().eq(b.to_lowercase())
}

fn match_at(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    while pi < p.len() {
        match p[pi] {
            '*' => {
                // Collapse runs of '*'.
                while pi < p.len() && p[pi] == '*' {
                    pi += 1;
                }
                if pi == p.len() {
                    return true;
                }
                for skip in ti..=t.len() {
                    if match_at(p, pi, t, skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= t.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            c => {
                if ti >= t.len() || !eq_ci(c, t[ti]) {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == t.len()
}

/// Pick the assets from `assets` that match any of the repo's patterns.
/// With no patterns, nothing is auto-picked (download-only by explicit
/// choice is the safe default — the user adds a rule or picks by hand).
pub fn select_assets(patterns: &[String], assets: &[String]) -> Vec<String> {
    assets
        .iter()
        .filter(|a| patterns.iter().any(|p| glob_match(p, a)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_windows_asset_rules() {
        assert!(glob_match("*-win64.zip", "App-v1.0-Win64.ZIP"));
        assert!(glob_match("*.exe", "Setup.exe"));
        assert!(!glob_match("*.exe", "Setup.exe.sha256"));
        assert!(glob_match("*x64*Setup*", "App-v2-x64-Setup.exe"));
        assert!(glob_match("app-?.zip", "app-1.zip"));
        assert!(!glob_match("app-?.zip", "app-12.zip"));
        assert!(glob_match("*", "anything.tar.gz"));
        assert!(!glob_match("*.dmg", "app.zip"));
    }

    #[test]
    fn select_assets_uses_any_pattern() {
        let assets = vec![
            "a-linux.tar.gz".to_string(),
            "a-win64.zip".to_string(),
            "a.dmg".to_string(),
        ];
        let picked = select_assets(&["*win64*".to_string()], &assets);
        assert_eq!(picked, vec!["a-win64.zip".to_string()]);
        assert!(select_assets(&[], &assets).is_empty());
    }
}
