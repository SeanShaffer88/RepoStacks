//! Parsing and validation of `owner/repo` references.

use std::fmt;

/// A GitHub repository reference. Display case is preserved as typed, but
/// identity is case-insensitive — GitHub treats `Sharkdp/bat` and
/// `sharkdp/bat` as the same repository, and so does the catalogue.
#[derive(Debug, Clone)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// Lowercased identity key used for equality, hashing, and ordering.
    pub fn key(&self) -> (String, String) {
        (self.owner.to_lowercase(), self.name.to_lowercase())
    }
}

impl PartialEq for RepoRef {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl Eq for RepoRef {}

impl PartialOrd for RepoRef {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RepoRef {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}
impl std::hash::Hash for RepoRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key().hash(state);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRefError(pub String);

impl fmt::Display for RepoRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for RepoRefError {}

impl RepoRef {
    /// Accepts `owner/repo`, `github.com/owner/repo`, or a full
    /// `https://github.com/owner/repo` URL (with optional trailing `/` or `.git`).
    pub fn parse(input: &str) -> Result<RepoRef, RepoRefError> {
        let s = input.trim();
        let s = s.strip_suffix('/').unwrap_or(s);
        let path = if let Some(rest) = s
            .strip_prefix("https://github.com/")
            .or_else(|| s.strip_prefix("http://github.com/"))
            .or_else(|| s.strip_prefix("github.com/"))
        {
            rest
        } else {
            s
        };
        let path = path.strip_suffix('/').unwrap_or(path);
        let path = path.strip_suffix(".git").unwrap_or(path);
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 2 {
            return Err(RepoRefError(format!("expected owner/repo, got '{input}'")));
        }
        let (owner, name) = (parts[0], parts[1]);
        validate_owner(owner)?;
        validate_name(name)?;
        Ok(RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn api_release_by_tag_url(&self, tag: &str) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            self.owner, self.name, tag
        )
    }

    pub fn releases_atom_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases.atom",
            self.owner, self.name
        )
    }

    pub fn tags_atom_url(&self) -> String {
        format!("https://github.com/{}/{}/tags.atom", self.owner, self.name)
    }

    pub fn api_repo_url(&self) -> String {
        format!("https://api.github.com/repos/{}/{}", self.owner, self.name)
    }

    pub fn latest_release_url(&self) -> String {
        format!("{}/releases/latest", self.api_repo_url())
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

fn validate_owner(owner: &str) -> Result<(), RepoRefError> {
    // GitHub logins: 1-39 chars, alphanumeric or hyphen, no leading/trailing hyphen.
    if owner.is_empty() || owner.len() > 39 {
        return Err(RepoRefError(format!("owner '{owner}' has a bad length")));
    }
    if owner.starts_with('-') || owner.ends_with('-') {
        return Err(RepoRefError(format!(
            "owner '{owner}' may not start or end with a hyphen"
        )));
    }
    if !owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(RepoRefError(format!(
            "owner '{owner}' has characters GitHub does not allow"
        )));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), RepoRefError> {
    if name.is_empty() || name.len() > 100 || name == "." || name == ".." {
        return Err(RepoRefError(format!("repo name '{name}' is not valid")));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(RepoRefError(format!(
            "repo name '{name}' has characters GitHub does not allow"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_url_forms() {
        let a = RepoRef::parse("rust-lang/rust").unwrap();
        assert_eq!(a.full_name(), "rust-lang/rust");
        assert_eq!(
            RepoRef::parse("https://github.com/rust-lang/rust").unwrap(),
            a
        );
        assert_eq!(RepoRef::parse("github.com/rust-lang/rust/").unwrap(), a);
        assert_eq!(
            RepoRef::parse("https://github.com/rust-lang/rust.git").unwrap(),
            a
        );
    }

    #[test]
    fn builds_feed_and_api_urls() {
        let r = RepoRef::parse("SeanShaffer88/RepoStacks").unwrap();
        assert_eq!(
            r.releases_atom_url(),
            "https://github.com/SeanShaffer88/RepoStacks/releases.atom"
        );
        assert_eq!(
            r.latest_release_url(),
            "https://api.github.com/repos/SeanShaffer88/RepoStacks/releases/latest"
        );
    }

    #[test]
    fn rejects_bad_refs() {
        for bad in [
            "",
            "noslash",
            "a/b/c",
            "-bad/repo",
            "bad-/repo",
            "a/b c",
            "a/.",
            "/repo",
            "owner/",
        ] {
            assert!(RepoRef::parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}
