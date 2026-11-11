//! Rate-limit policy — the corrected design.
//!
//! GitHub's rules (docs, "Rate limits for the REST API" / "Best practices"):
//! - Unauthenticated REST calls: 60/hour per IP. With a token: 5,000/hour.
//! - A conditional request answered `304 Not Modified` does **not** count
//!   against the primary limit **only when the request was correctly
//!   authorized**. An unauthenticated 304 still decrements the quota —
//!   ETags save bandwidth for anonymous callers, not quota.
//!
//! RepoStacks therefore treats the two modes differently instead of
//! assuming ETags are free for everyone:
//! - Release detection always uses the Atom feeds (no REST quota at all).
//! - Metadata (stars, description, topics) is fetched rarely and on demand
//!   when anonymous, and may be polled with ETags only when a token exists.
//! - Asset lists cost exactly one REST call per actual new release.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    Anonymous,
    Token,
}

impl Auth {
    pub fn hourly_rest_limit(&self) -> u32 {
        match self {
            Auth::Anonymous => 60,
            Auth::Token => 5_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePolicy {
    pub auth: Auth,
    /// How often the Atom release feed may be polled, in seconds.
    pub atom_poll_secs: u64,
    /// Minimum age of cached metadata before a background REST refresh.
    pub metadata_refresh_secs: u64,
}

impl CachePolicy {
    pub fn for_auth(auth: Auth) -> CachePolicy {
        match auth {
            // Anonymous: lean on Atom (free) + long metadata cache.
            Auth::Anonymous => CachePolicy {
                auth,
                atom_poll_secs: 3_600,
                metadata_refresh_secs: 24 * 3_600,
            },
            // Token: ETag polling is genuinely free on a 304, so metadata
            // can refresh much more often at no quota cost when unchanged.
            Auth::Token => CachePolicy {
                auth,
                atom_poll_secs: 3_600,
                metadata_refresh_secs: 6 * 3_600,
            },
        }
    }

    /// Does a REST response consume one unit of the hourly quota?
    /// `status` is the HTTP status code; a 304 is free **only** with a token.
    pub fn response_counts_against_quota(&self, status: u16) -> bool {
        if status == 304 && self.auth == Auth::Token {
            return false;
        }
        (200..400).contains(&status) || status == 403 || status == 429
    }

    /// May cached metadata be refreshed by a background REST call yet?
    pub fn metadata_refresh_due(&self, last_fetch_unix: u64, now_unix: u64) -> bool {
        now_unix.saturating_sub(last_fetch_unix) >= self.metadata_refresh_secs
    }

    /// The one REST call a new release is allowed to cost: its asset list.
    pub fn rest_calls_for_new_release(&self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_304_still_counts() {
        // Regression guard for the blueprint's §3.3 error: without a token a
        // 304 is NOT free.
        let p = CachePolicy::for_auth(Auth::Anonymous);
        assert!(p.response_counts_against_quota(304));
        assert!(p.response_counts_against_quota(200));
        assert_eq!(p.auth.hourly_rest_limit(), 60);
    }

    #[test]
    fn authenticated_304_is_free() {
        let p = CachePolicy::for_auth(Auth::Token);
        assert!(!p.response_counts_against_quota(304));
        assert!(p.response_counts_against_quota(200));
        assert_eq!(p.auth.hourly_rest_limit(), 5_000);
    }

    #[test]
    fn anonymous_metadata_cache_is_longer() {
        let anon = CachePolicy::for_auth(Auth::Anonymous);
        let token = CachePolicy::for_auth(Auth::Token);
        assert!(anon.metadata_refresh_secs > token.metadata_refresh_secs);
        assert!(!anon.metadata_refresh_due(1_000, 1_000 + 3_600));
        assert!(anon.metadata_refresh_due(1_000, 1_000 + anon.metadata_refresh_secs));
    }
}
