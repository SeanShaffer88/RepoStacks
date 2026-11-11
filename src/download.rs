//! The download engine: plan (pick the right asset), fetch (with Range
//! resume through the `.part` file), verify (SHA-256 against the digest
//! GitHub publishes), and land the file where the catalogue says it lives:
//! `<download root>/<owner>-<repo>/<tag>/<asset>`.

use crate::catalog::RepoEntry;
use crate::glob_match::{glob_match, select_assets};
use crate::http::Downloader;
use crate::net::ReleaseAsset;
use crate::queue::{DownloadJob, JobStatus, Queue};
use crate::sha256::sha256_file_hex;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedDownload {
    pub asset_name: String,
    pub url: String,
    pub size: u64,
    /// Expected SHA-256 hex from the release's published digest, if any.
    pub expected_sha256: String,
    /// Why this asset was picked, in plain words — surfaces show it so
    /// the auto-pick is never a mystery.
    pub reason: String,
    pub part_path: PathBuf,
    pub final_path: PathBuf,
}

/// Download URLs come from the release API, but bytes should only ever
/// be fetched from GitHub's own hosts over https. Anything else means
/// the data was tampered with (or the API changed shape) — refuse.
pub fn validate_download_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("download URL is not https: {url}"))?;
    let host = rest.split('/').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    if host == "github.com" || host.ends_with(".githubusercontent.com") {
        Ok(())
    } else {
        Err(format!(
            "download URL host '{host}' is not a GitHub host — refusing to download"
        ))
    }
}

fn dest_dir(root: &std::path::Path, entry: &RepoEntry, tag: &str) -> PathBuf {
    root.join(format!("{}-{}", entry.repo.owner, entry.repo.name))
        .join(tag)
}

/// Choose the asset for a volume and lay out its paths. The volume's own
/// glob rules win; without rules, a Windows-first preference order applies
/// (this is a Windows library): windows/win64 builds first, archives and
/// installers ahead of bare binaries, other operating systems last.
pub fn plan_download(
    entry: &RepoEntry,
    tag: &str,
    assets: &[ReleaseAsset],
    root: &std::path::Path,
) -> Result<PlannedDownload, String> {
    if assets.is_empty() {
        return Err(format!("{} {tag} has no downloadable assets", entry.repo));
    }
    let names: Vec<String> = assets.iter().map(|a| a.name.clone()).collect();
    let (chosen_name, reason) = if !entry.asset_patterns.is_empty() {
        let picked = select_assets(&entry.asset_patterns, &names);
        match picked.first() {
            Some(n) => {
                let pattern = entry
                    .asset_patterns
                    .iter()
                    .find(|p| glob_match(p, n))
                    .cloned()
                    .unwrap_or_default();
                (n.clone(), format!("matches its asset rule '{pattern}'"))
            }
            None => {
                return Err(format!(
                    "no asset of {} {tag} matches its rules ({})",
                    entry.repo,
                    entry.asset_patterns.join(", ")
                ))
            }
        }
    } else {
        let best = names
            .iter()
            .max_by_key(|n| windows_score(n))
            .expect("assets non-empty")
            .clone();
        let parts = score_parts(&best);
        let total: i32 = parts.iter().map(|(s, _)| s).sum();
        let detail = if parts.is_empty() {
            "no signals in the name".to_string()
        } else {
            parts
                .iter()
                .map(|(s, label)| format!("{s:+} {label}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        (best, format!("auto-pick, score {total} ({detail})"))
    };
    let asset = assets
        .iter()
        .find(|a| a.name == chosen_name)
        .expect("chosen from the same list");
    validate_download_url(&asset.url)?;
    let dir = dest_dir(root, entry, tag);
    let expected = asset
        .digest
        .strip_prefix("sha256:")
        .unwrap_or(&asset.digest)
        .to_lowercase();
    Ok(PlannedDownload {
        asset_name: asset.name.clone(),
        url: asset.url.clone(),
        size: asset.size,
        expected_sha256: if asset.digest.is_empty() {
            String::new()
        } else {
            expected
        },
        reason,
        part_path: dir.join(format!("{}.part", asset.name)),
        final_path: dir.join(&asset.name),
    })
}

/// The auto-pick score as labelled parts, so the total can be explained.
/// A library wants the program itself: setup programs and MSI packages
/// score down (the file in the library should be the thing, not its
/// installer), other operating systems and checksum files score out.
fn score_parts(name: &str) -> Vec<(i32, &'static str)> {
    let n = name.to_lowercase();
    let mut parts = Vec::new();
    if n.contains("windows") {
        parts.push((4, "windows"));
    }
    if n.contains("win64") || n.contains("win-x64") {
        parts.push((3, "win64"));
    } else if n.contains("win") {
        parts.push((1, "win"));
    }
    if n.contains("x86_64") || n.contains("amd64") || n.contains("x64") {
        parts.push((2, "64-bit"));
    }
    if n.ends_with(".zip") || n.ends_with(".exe") {
        parts.push((1, "archive/program file"));
    }
    if n.contains("linux") || n.contains("darwin") || n.contains("macos") || n.contains("apple") {
        parts.push((-4, "other OS"));
    }
    if n.contains("arm") || n.contains("aarch") {
        parts.push((-2, "ARM build"));
    }
    if n.contains("checksum") || n.contains("sha256") || n.ends_with(".sig") || n.ends_with(".asc")
    {
        parts.push((-8, "checksum/signature file"));
    }
    if n.contains("setup") || n.contains("install") {
        parts.push((-3, "setup program"));
    }
    if n.ends_with(".msi") {
        parts.push((-3, "MSI package"));
    }
    parts
}

fn windows_score(name: &str) -> i32 {
    score_parts(name).iter().map(|(s, _)| s).sum()
}

/// Plan + enqueue. Returns the job id (an identical active job is reused).
pub fn enqueue_download(
    queue: &mut Queue,
    entry: &RepoEntry,
    tag: &str,
    assets: &[ReleaseAsset],
    root: &std::path::Path,
) -> Result<u64, String> {
    let plan = plan_download(entry, tag, assets, root)?;
    let id = queue.enqueue(entry.repo.clone(), tag, &plan.asset_name, &plan.url);
    if let Some(job) = queue.get_mut(id) {
        job.bytes_total = plan.size;
        if job.checksum_sha256.is_empty() {
            job.checksum_sha256 = plan.expected_sha256.clone();
        }
    }
    Ok(id)
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobOutcome {
    pub id: u64,
    pub status: JobStatus,
    pub bytes_done: u64,
    pub sha256: String,
    pub error: String,
    pub final_path: PathBuf,
}

/// Execute one job from a snapshot of its fields. Pure with respect to the
/// queue and catalogue — `run_pending` applies the outcome — so worker
/// threads never share mutable state.
pub fn execute_job<D: Downloader>(
    job: &DownloadJob,
    downloader: &D,
    root: &std::path::Path,
    token: Option<&str>,
) -> JobOutcome {
    let dir = root
        .join(format!("{}-{}", job.repo.owner, job.repo.name))
        .join(&job.tag);
    let final_path = dir.join(&job.asset_name);
    let part_path = dir.join(format!("{}.part", job.asset_name));
    let fail = |error: String| JobOutcome {
        id: job.id,
        status: JobStatus::Failed,
        bytes_done: job.bytes_done,
        sha256: String::new(),
        error,
        final_path: final_path.clone(),
    };
    // Jobs persist on disk; re-validate the URL at execution time too,
    // so a queue file written (or tampered with) earlier cannot steer
    // the downloader to an arbitrary host.
    if let Err(e) = validate_download_url(&job.url) {
        return fail(e);
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(format!("create download folder: {e}"));
    }
    let resume = part_path.exists();
    let bytes = match downloader.download(&job.url, &part_path, resume, token) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    if job.bytes_total > 0 && bytes != job.bytes_total {
        // A short (or overlong) transfer is not the published file. The
        // .part stays on disk: a retry resumes from it instead of
        // starting over.
        return fail(format!(
            "size mismatch: expected {} bytes, got {bytes} — partial file kept for resume",
            job.bytes_total
        ));
    }
    let sha = match sha256_file_hex(&part_path) {
        Ok(s) => s,
        Err(e) => return fail(format!("read downloaded file: {e}")),
    };
    if !job.checksum_sha256.is_empty() && job.checksum_sha256.to_lowercase() != sha {
        // Bytes that fail verification are deleted, never installed —
        // a mismatch means corruption or tampering, and the job says why.
        let _ = std::fs::remove_file(&part_path);
        return fail(format!(
            "checksum mismatch: expected {}, got {} — file deleted",
            job.checksum_sha256, sha
        ));
    }
    if let Err(e) = std::fs::rename(&part_path, &final_path) {
        return fail(format!("move download into place: {e}"));
    }
    JobOutcome {
        id: job.id,
        status: JobStatus::Done,
        bytes_done: bytes,
        sha256: sha,
        error: String::new(),
        final_path,
    }
}

/// Run every queued job, at most `concurrency` at a time, applying each
/// outcome to the queue and (on success) marking the volume installed at
/// the job's tag. Returns one outcome per job run.
pub fn run_pending<D: Downloader + Sync>(
    queue: &mut Queue,
    catalog: &mut crate::catalog::Catalog,
    downloader: &D,
    root: &std::path::Path,
    token: Option<&str>,
    concurrency: usize,
) -> Vec<JobOutcome> {
    let ids: Vec<u64> = queue
        .jobs
        .iter()
        .filter(|j| j.status == JobStatus::Queued)
        .map(|j| j.id)
        .collect();
    let mut outcomes = Vec::new();
    let stride = concurrency.max(1);
    for chunk in ids.chunks(stride) {
        let snapshots: Vec<DownloadJob> = chunk
            .iter()
            .filter_map(|id| queue.jobs.iter().find(|j| j.id == *id).cloned())
            .collect();
        for job in &snapshots {
            if let Some(j) = queue.get_mut(job.id) {
                j.status = JobStatus::Downloading;
                j.error.clear();
            }
        }
        let chunk_outcomes: Vec<JobOutcome> = std::thread::scope(|scope| {
            let handles: Vec<_> = snapshots
                .iter()
                .map(|job| scope.spawn(|| execute_job(job, downloader, root, token)))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("download worker panicked"))
                .collect()
        });
        for outcome in chunk_outcomes {
            if let Some(job) = queue.get_mut(outcome.id) {
                job.status = outcome.status;
                job.bytes_done = outcome.bytes_done;
                if outcome.status == JobStatus::Done {
                    job.checksum_sha256 = outcome.sha256.clone();
                    job.bytes_total = outcome.bytes_done;
                }
                job.error = outcome.error.clone();
            }
            if outcome.status == JobStatus::Done {
                if let Some(job) = queue.jobs.iter().find(|j| j.id == outcome.id) {
                    let (repo, tag) = (job.repo.clone(), job.tag.clone());
                    if let Some(entry) = catalog.get_mut(&repo) {
                        entry.installed_tag = tag;
                    }
                }
            }
            outcomes.push(outcome);
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::repo_ref::RepoRef;
    use std::sync::Mutex;

    struct FakeDownloader {
        payload: Mutex<Vec<u8>>,
        calls: Mutex<Vec<(String, bool)>>,
    }
    impl Downloader for FakeDownloader {
        fn download(
            &self,
            url: &str,
            dest: &std::path::Path,
            resume: bool,
            _token: Option<&str>,
        ) -> Result<u64, String> {
            self.calls.lock().unwrap().push((url.to_string(), resume));
            let data = self.payload.lock().unwrap().clone();
            if resume {
                // Simulate a server honouring Range: append the remainder
                // after the bytes already present (half, say).
                let have = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0) as usize;
                let mut all = std::fs::read(dest).unwrap_or_default();
                all.extend_from_slice(&data[have.min(data.len())..]);
                std::fs::write(dest, &all).unwrap();
                Ok(all.len() as u64)
            } else {
                std::fs::write(dest, &data).unwrap();
                Ok(data.len() as u64)
            }
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("repostacks-dl-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn asset(name: &str, digest: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            url: format!("https://github.com/o/r/releases/download/v1/{name}"),
            size: 0,
            digest: digest.into(),
        }
    }

    #[test]
    fn plan_prefers_windows_asset_and_honours_rules() {
        let mut e = RepoEntry::new(RepoRef::parse("o/r").unwrap());
        let assets = vec![
            asset("r-linux.tar.gz", ""),
            asset("r-windows-x86_64.zip", ""),
            asset("checksums.txt", ""),
        ];
        let plan = plan_download(&e, "v1", &assets, std::path::Path::new("/dl")).unwrap();
        assert_eq!(plan.asset_name, "r-windows-x86_64.zip");
        assert!(plan.reason.contains("auto-pick"), "reason: {}", plan.reason);
        assert!(plan.reason.contains("windows"), "reason: {}", plan.reason);
        e.asset_patterns = vec!["*-linux.tar.gz".into()];
        let plan2 = plan_download(&e, "v1", &assets, std::path::Path::new("/dl")).unwrap();
        assert_eq!(plan2.asset_name, "r-linux.tar.gz");
        assert!(
            plan2.reason.contains("*-linux.tar.gz"),
            "reason: {}",
            plan2.reason
        );
        e.asset_patterns = vec!["*.dmg".into()];
        assert!(plan_download(&e, "v1", &assets, std::path::Path::new("/dl")).is_err());
    }

    #[test]
    fn plan_prefers_the_program_over_its_setup() {
        let e = RepoEntry::new(RepoRef::parse("o/r").unwrap());
        let assets = vec![
            asset("r-windows-setup.exe", ""),
            asset("r-windows-x64.zip", ""),
        ];
        let plan = plan_download(&e, "v1", &assets, std::path::Path::new("/dl")).unwrap();
        assert_eq!(plan.asset_name, "r-windows-x64.zip");
    }

    #[test]
    fn download_urls_must_be_github_https() {
        assert!(validate_download_url("https://github.com/o/r/releases/download/v1/a.zip").is_ok());
        assert!(validate_download_url("https://objects.githubusercontent.com/x").is_ok());
        assert!(validate_download_url("http://github.com/a.zip").is_err());
        assert!(validate_download_url("https://evil.example/a.zip").is_err());
        assert!(validate_download_url("https://github.com.evil.example/a.zip").is_err());
    }

    #[test]
    fn engine_fails_a_short_download_and_keeps_the_part() {
        let root = temp_root("short");
        let payload = b"half".to_vec();
        let assets = vec![ReleaseAsset {
            name: "r-windows.zip".into(),
            url: "https://github.com/o/r/releases/download/v1/r-windows.zip".into(),
            size: 100, // the listing promises 100 bytes; only 4 arrive
            digest: String::new(),
        }];
        let mut cat = Catalog::default();
        cat.add(RepoRef::parse("o/r").unwrap());
        let entry = cat.get(&RepoRef::parse("o/r").unwrap()).unwrap().clone();
        let mut queue = Queue::default();
        enqueue_download(&mut queue, &entry, "v1", &assets, &root).unwrap();
        let dl = FakeDownloader {
            payload: Mutex::new(payload),
            calls: Mutex::new(Vec::new()),
        };
        let outcomes = run_pending(&mut queue, &mut cat, &dl, &root, None, 1);
        assert_eq!(outcomes[0].status, JobStatus::Failed);
        assert!(outcomes[0].error.contains("size mismatch"));
        assert!(!outcomes[0].final_path.exists());
        let part = root.join("o-r").join("v1").join("r-windows.zip.part");
        assert!(part.exists(), "the partial file stays for a resume");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn engine_downloads_verifies_and_marks_installed() {
        let root = temp_root("ok");
        let payload = b"hello repostacks".to_vec();
        let digest = format!("sha256:{}", crate::sha256::sha256_hex(&payload));
        let assets = vec![ReleaseAsset {
            name: "r-windows.zip".into(),
            url: "https://github.com/o/r/releases/download/v1/r-windows.zip".into(),
            size: payload.len() as u64,
            digest,
        }];
        let mut cat = Catalog::default();
        cat.add(RepoRef::parse("o/r").unwrap());
        let entry = cat.get(&RepoRef::parse("o/r").unwrap()).unwrap().clone();
        let mut queue = Queue::default();
        let id = enqueue_download(&mut queue, &entry, "v1", &assets, &root).unwrap();
        let dl = FakeDownloader {
            payload: Mutex::new(payload),
            calls: Mutex::new(Vec::new()),
        };
        let outcomes = run_pending(&mut queue, &mut cat, &dl, &root, None, 1);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, JobStatus::Done);
        assert!(outcomes[0].final_path.exists());
        assert_eq!(queue.jobs[0].status, JobStatus::Done);
        assert_eq!(queue.jobs[0].id, id);
        assert_eq!(
            cat.get(&RepoRef::parse("o/r").unwrap())
                .unwrap()
                .installed_tag,
            "v1",
            "a landed download marks the volume installed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn engine_resumes_from_a_part_file() {
        let root = temp_root("resume");
        let payload = b"0123456789abcdef".to_vec();
        let digest = format!("sha256:{}", crate::sha256::sha256_hex(&payload));
        let assets = vec![ReleaseAsset {
            name: "r-windows.zip".into(),
            url: "https://github.com/o/r/releases/download/v1/r-windows.zip".into(),
            size: payload.len() as u64,
            digest,
        }];
        let mut cat = Catalog::default();
        cat.add(RepoRef::parse("o/r").unwrap());
        let entry = cat.get(&RepoRef::parse("o/r").unwrap()).unwrap().clone();
        let mut queue = Queue::default();
        enqueue_download(&mut queue, &entry, "v1", &assets, &root).unwrap();
        // Seed a .part holding the first 6 bytes, as an interrupted run would.
        let dir = root.join("o-r").join("v1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("r-windows.zip.part"), &payload[..6]).unwrap();
        let dl = FakeDownloader {
            payload: Mutex::new(payload.clone()),
            calls: Mutex::new(Vec::new()),
        };
        let outcomes = run_pending(&mut queue, &mut cat, &dl, &root, None, 1);
        assert_eq!(outcomes[0].status, JobStatus::Done);
        assert_eq!(std::fs::read(&outcomes[0].final_path).unwrap(), payload);
        let calls = dl.calls.lock().unwrap();
        assert!(
            calls[0].1,
            "the engine must ask the transport to resume when a .part exists"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn engine_rejects_checksum_mismatch_and_deletes() {
        let root = temp_root("bad");
        let assets = vec![ReleaseAsset {
            name: "r-windows.zip".into(),
            url: "https://github.com/o/r/releases/download/v1/r-windows.zip".into(),
            size: 3,
            digest: format!("sha256:{}", "0".repeat(64)),
        }];
        let mut cat = Catalog::default();
        cat.add(RepoRef::parse("o/r").unwrap());
        let entry = cat.get(&RepoRef::parse("o/r").unwrap()).unwrap().clone();
        let mut queue = Queue::default();
        enqueue_download(&mut queue, &entry, "v1", &assets, &root).unwrap();
        let dl = FakeDownloader {
            payload: Mutex::new(b"abc".to_vec()),
            calls: Mutex::new(Vec::new()),
        };
        let outcomes = run_pending(&mut queue, &mut cat, &dl, &root, None, 1);
        assert_eq!(outcomes[0].status, JobStatus::Failed);
        assert!(outcomes[0].error.contains("checksum mismatch"));
        assert!(!outcomes[0].final_path.exists());
        assert!(cat
            .get(&RepoRef::parse("o/r").unwrap())
            .unwrap()
            .installed_tag
            .is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
