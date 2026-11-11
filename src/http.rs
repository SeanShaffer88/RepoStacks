//! HTTP fetching behind a trait, so network operations are testable with a
//! scripted fetcher and the live transport can be swapped without touching
//! the operations.
//!
//! The live transport shells out to the system `curl` (curl.exe ships with
//! Windows 10 1803+ and Windows 11; curl is standard on Linux dev machines).
//! That keeps the core at zero crate dependencies. The token, when present,
//! is passed as a header by the operations layer and is never logged or
//! persisted by this module.

#[derive(Debug, Clone, PartialEq)]
pub struct FetchResponse {
    pub status: u16,
    pub etag: String,
    pub body: String,
}

/// Build a command that never flashes a console window. The window app
/// has no console of its own, so on Windows every plain child process
/// gets a fresh command-prompt window — one per check, refresh, and
/// download. CREATE_NO_WINDOW suppresses it; elsewhere this is just a
/// plain command.
pub fn hidden_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub trait Fetcher {
    /// GET `url`. `etag` sends `If-None-Match`; `token` sends
    /// `Authorization: Bearer …`. A 304 response is a normal Ok result.
    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        token: Option<&str>,
    ) -> Result<FetchResponse, String>;
}

pub struct CurlFetcher {
    pub program: String,
}

impl Default for CurlFetcher {
    fn default() -> Self {
        CurlFetcher {
            program: "curl".to_string(),
        }
    }
}

impl Fetcher for CurlFetcher {
    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
        token: Option<&str>,
    ) -> Result<FetchResponse, String> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir();
        let body_path = dir.join(format!("repostacks-body-{stamp}.tmp"));
        let head_path = dir.join(format!("repostacks-head-{stamp}.tmp"));
        let mut cmd = hidden_command(&self.program);
        cmd.args([
            "-sS",
            "--fail-with-body",
            "--max-time",
            "30",
            "-A",
            concat!("RepoStacks/", env!("CARGO_PKG_VERSION")),
            "-o",
        ])
        .arg(&body_path)
        .arg("-D")
        .arg(&head_path)
        .arg("-w")
        .arg("%{http_code}");
        if let Some(tag) = etag {
            if !tag.is_empty() {
                cmd.arg("-H").arg(format!("If-None-Match: {tag}"));
            }
        }
        if let Some(tok) = token {
            if !tok.is_empty() {
                cmd.arg("-H").arg(format!("Authorization: Bearer {tok}"));
            }
        }
        cmd.arg(url);
        let out = cmd
            .output()
            .map_err(|e| format!("could not run {} (is curl installed?): {e}", self.program))?;
        let status_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // --fail-with-body makes HTTP errors exit non-zero, but the status is
        // still printed; prefer the printed status whenever we have one.
        let status: u16 = status_text.parse().unwrap_or(0);
        let headers = std::fs::read_to_string(&head_path).unwrap_or_default();
        let body = std::fs::read_to_string(&body_path).unwrap_or_default();
        let _ = std::fs::remove_file(&body_path);
        let _ = std::fs::remove_file(&head_path);
        if status == 0 {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() {
                format!("fetch of {url} failed")
            } else {
                format!("fetch of {url} failed: {err}")
            });
        }
        Ok(FetchResponse {
            status,
            etag: parse_etag(&headers),
            body,
        })
    }
}

fn parse_etag(headers: &str) -> String {
    // Header dump may hold several response blocks (redirects); the last
    // ETag wins.
    let mut etag = String::new();
    for line in headers.lines() {
        let lower = line.to_lowercase();
        if let Some(rest) = lower.strip_prefix("etag:") {
            let start = line.len() - rest.len();
            etag = line[start..].trim().to_string();
        }
    }
    etag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_last_etag_from_header_dump() {
        let h = "HTTP/2 200\r\netag: \"abc\"\r\n\r\nHTTP/2 200\r\nETag: \"def\"\r\n";
        assert_eq!(parse_etag(h), "\"def\"");
        assert_eq!(parse_etag("HTTP/2 304\r\n"), "");
    }
}

/// Byte-moving transport for the download engine, behind a trait for the
/// same reason as `Fetcher`: the engine is testable with a fake.
pub trait Downloader {
    /// Download `url` to `dest` (a `.part` file). When `resume` is true and
    /// `dest` already holds bytes, continue from that offset. Returns the
    /// total bytes now in `dest`.
    fn download(
        &self,
        url: &str,
        dest: &std::path::Path,
        resume: bool,
        token: Option<&str>,
    ) -> Result<u64, String>;
}

pub struct CurlDownloader {
    pub program: String,
}

impl Default for CurlDownloader {
    fn default() -> Self {
        CurlDownloader {
            program: "curl".to_string(),
        }
    }
}

impl Downloader for CurlDownloader {
    fn download(
        &self,
        url: &str,
        dest: &std::path::Path,
        resume: bool,
        token: Option<&str>,
    ) -> Result<u64, String> {
        let attempt = |resume: bool| -> Result<(), String> {
            let mut cmd = hidden_command(&self.program);
            cmd.args([
                "-sS",
                "-L",
                "--fail",
                "--max-time",
                "3600",
                "-A",
                concat!("RepoStacks/", env!("CARGO_PKG_VERSION")),
                "-o",
            ])
            .arg(dest);
            if resume {
                cmd.arg("-C").arg("-");
            }
            if let Some(tok) = token {
                if !tok.is_empty() {
                    cmd.arg("-H").arg(format!("Authorization: Bearer {tok}"));
                }
            }
            cmd.arg(url);
            let out = cmd
                .output()
                .map_err(|e| format!("could not run {}: {e}", self.program))?;
            if out.status.success() {
                Ok(())
            } else {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(if err.is_empty() {
                    format!("download of {url} failed")
                } else {
                    err
                })
            }
        };
        let existing = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        if resume && existing > 0 {
            match attempt(true) {
                Ok(()) => {}
                // A server that cannot resume makes curl fail; start over
                // once rather than failing the job.
                Err(_) => {
                    let _ = std::fs::remove_file(dest);
                    attempt(false)?;
                }
            }
        } else {
            attempt(false)?;
        }
        std::fs::metadata(dest)
            .map(|m| m.len())
            .map_err(|e| format!("download produced no file: {e}"))
    }
}
