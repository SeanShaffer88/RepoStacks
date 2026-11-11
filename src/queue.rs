//! The acquisitions queue: download jobs with pause/resume state.
//! The engine that moves bytes lands with the network slice; this module is
//! the state machine every surface (CLI, window, scheduler) drives.

use crate::json::Json;
use crate::repo_ref::RepoRef;

pub const QUEUE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Downloading,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn label(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Downloading => "downloading",
            JobStatus::Paused => "paused",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadJob {
    pub id: u64,
    pub repo: RepoRef,
    pub tag: String,
    pub asset_name: String,
    pub url: String,
    pub status: JobStatus,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub checksum_sha256: String,
    pub error: String,
}

impl DownloadJob {
    pub fn progress_percent(&self) -> f64 {
        if self.bytes_total == 0 {
            return if self.status == JobStatus::Done {
                100.0
            } else {
                0.0
            };
        }
        (self.bytes_done as f64 / self.bytes_total as f64 * 100.0).min(100.0)
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            JobStatus::Queued | JobStatus::Downloading | JobStatus::Paused
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    pub jobs: Vec<DownloadJob>,
    next_id: u64,
}

impl Queue {
    /// Enqueue a download; an identical active job (same repo, tag, asset)
    /// is returned instead of duplicated.
    pub fn enqueue(&mut self, repo: RepoRef, tag: &str, asset_name: &str, url: &str) -> u64 {
        if let Some(j) = self
            .jobs
            .iter()
            .find(|j| j.repo == repo && j.tag == tag && j.asset_name == asset_name && j.is_active())
        {
            return j.id;
        }
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.push(DownloadJob {
            id,
            repo,
            tag: tag.to_string(),
            asset_name: asset_name.to_string(),
            url: url.to_string(),
            status: JobStatus::Queued,
            bytes_done: 0,
            bytes_total: 0,
            checksum_sha256: String::new(),
            error: String::new(),
        });
        id
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut DownloadJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn pause(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(j) if j.status == JobStatus::Downloading || j.status == JobStatus::Queued => {
                j.status = JobStatus::Paused;
                true
            }
            _ => false,
        }
    }

    pub fn resume(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(j) if j.status == JobStatus::Paused => {
                j.status = JobStatus::Queued;
                true
            }
            _ => false,
        }
    }

    pub fn cancel(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(j) if j.is_active() => {
                j.status = JobStatus::Cancelled;
                true
            }
            _ => false,
        }
    }

    /// Requeue a failed job, clearing its error. The `.part` file (if the
    /// failure left one) makes the retry resume instead of restarting.
    pub fn retry(&mut self, id: u64) -> bool {
        match self.get_mut(id) {
            Some(j) if j.status == JobStatus::Failed => {
                j.status = JobStatus::Queued;
                j.error.clear();
                true
            }
            _ => false,
        }
    }

    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.is_active()).count()
    }

    /// Drop finished jobs (done / failed / cancelled), keeping active ones.
    pub fn clear_finished(&mut self) -> usize {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.is_active());
        before - self.jobs.len()
    }

    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            (
                "schema_version".into(),
                Json::Num(QUEUE_SCHEMA_VERSION as f64),
            ),
            ("next_id".into(), Json::Num(self.next_id as f64)),
            (
                "jobs".into(),
                Json::Arr(self.jobs.iter().map(job_to_json).collect()),
            ),
        ])
    }

    pub fn from_json(v: &Json) -> Result<Queue, String> {
        let version = v
            .get("schema_version")
            .and_then(Json::as_f64)
            .unwrap_or(0.0) as u32;
        if version != QUEUE_SCHEMA_VERSION {
            return Err(format!(
                "queue schema_version {version} is not supported (expected {QUEUE_SCHEMA_VERSION})"
            ));
        }
        let mut q = Queue::default();
        q.next_id = v.get("next_id").and_then(Json::as_f64).unwrap_or(0.0) as u64;
        if let Some(Json::Arr(items)) = v.get("jobs") {
            for item in items {
                if let Some(job) = job_from_json(item) {
                    q.next_id = q.next_id.max(job.id);
                    q.jobs.push(job);
                }
            }
        }
        Ok(q)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::persist::save_atomic(path, &self.to_json().to_string_pretty())
    }

    pub fn load(path: &std::path::Path) -> Result<Queue, String> {
        crate::persist::load_recovering(path, |text| {
            let v = crate::json::parse(text).map_err(|e| format!("parse queue: {e}"))?;
            Queue::from_json(&v)
        })
    }
}

fn status_from_label(s: &str) -> JobStatus {
    match s {
        "downloading" => JobStatus::Downloading,
        "paused" => JobStatus::Paused,
        "done" => JobStatus::Done,
        "failed" => JobStatus::Failed,
        "cancelled" => JobStatus::Cancelled,
        _ => JobStatus::Queued,
    }
}

fn job_to_json(j: &DownloadJob) -> Json {
    Json::Obj(vec![
        ("id".into(), Json::Num(j.id as f64)),
        ("repo".into(), Json::Str(j.repo.full_name())),
        ("tag".into(), Json::Str(j.tag.clone())),
        ("asset_name".into(), Json::Str(j.asset_name.clone())),
        ("url".into(), Json::Str(j.url.clone())),
        ("status".into(), Json::Str(j.status.label().into())),
        ("bytes_done".into(), Json::Num(j.bytes_done as f64)),
        ("bytes_total".into(), Json::Num(j.bytes_total as f64)),
        (
            "checksum_sha256".into(),
            Json::Str(j.checksum_sha256.clone()),
        ),
        ("error".into(), Json::Str(j.error.clone())),
    ])
}

fn job_from_json(v: &Json) -> Option<DownloadJob> {
    let repo = RepoRef::parse(v.get("repo")?.as_str()?).ok()?;
    let mut status = status_from_label(v.get("status").and_then(Json::as_str).unwrap_or("queued"));
    // A job caught mid-download by a shutdown restarts as queued; its
    // .part file on disk is what makes the restart cheap (Range resume).
    if status == JobStatus::Downloading {
        status = JobStatus::Queued;
    }
    Some(DownloadJob {
        id: v.get("id").and_then(Json::as_f64).unwrap_or(0.0) as u64,
        repo,
        tag: v.get("tag").and_then(Json::as_str).unwrap_or("").into(),
        asset_name: v
            .get("asset_name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into(),
        url: v.get("url").and_then(Json::as_str).unwrap_or("").into(),
        status,
        bytes_done: v.get("bytes_done").and_then(Json::as_f64).unwrap_or(0.0) as u64,
        bytes_total: v.get("bytes_total").and_then(Json::as_f64).unwrap_or(0.0) as u64,
        checksum_sha256: v
            .get("checksum_sha256")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into(),
        error: v.get("error").and_then(Json::as_str).unwrap_or("").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_json_round_trip_and_downloading_becomes_queued() {
        let mut q = Queue::default();
        let id = q.enqueue(
            RepoRef::parse("o/r").unwrap(),
            "v1",
            "a.zip",
            "https://x/a.zip",
        );
        {
            let j = q.get_mut(id).unwrap();
            j.status = JobStatus::Downloading;
            j.bytes_done = 10;
            j.checksum_sha256 = "abc".into();
        }
        let back = Queue::from_json(&crate::json::parse(&q.to_json().to_string_pretty()).unwrap())
            .unwrap();
        assert_eq!(back.jobs.len(), 1);
        assert_eq!(
            back.jobs[0].status,
            JobStatus::Queued,
            "interrupted downloads requeue"
        );
        assert_eq!(back.jobs[0].bytes_done, 10);
        assert_eq!(back.jobs[0].checksum_sha256, "abc");
        let bad = crate::json::parse("{\"schema_version\": 9}").unwrap();
        assert!(Queue::from_json(&bad).is_err());
    }

    #[test]
    fn enqueue_dedups_and_transitions_guard_state() {
        let mut q = Queue::default();
        let r = RepoRef::parse("o/r").unwrap();
        let id = q.enqueue(r.clone(), "v1", "a.zip", "https://example.invalid/a.zip");
        assert_eq!(
            q.enqueue(r, "v1", "a.zip", "https://example.invalid/a.zip"),
            id
        );
        assert_eq!(q.active_count(), 1);
        assert!(q.pause(id));
        assert!(q.resume(id));
        assert!(q.cancel(id));
        assert!(!q.resume(id), "cancelled jobs cannot resume");
        assert_eq!(q.active_count(), 0);
    }

    #[test]
    fn failed_jobs_retry_and_clear_their_error() {
        let mut q = Queue::default();
        let id = q.enqueue(
            RepoRef::parse("o/r").unwrap(),
            "v1",
            "a.zip",
            "https://x/a.zip",
        );
        {
            let j = q.get_mut(id).unwrap();
            j.status = JobStatus::Failed;
            j.error = "boom".into();
        }
        assert!(q.retry(id));
        let j = &q.jobs[0];
        assert_eq!(j.status, JobStatus::Queued);
        assert!(j.error.is_empty());
        assert!(!q.retry(id), "only failed jobs retry");
    }

    #[test]
    fn progress_is_bounded() {
        let mut q = Queue::default();
        let id = q.enqueue(RepoRef::parse("o/r").unwrap(), "v1", "a", "u");
        let j = q.get_mut(id).unwrap();
        j.bytes_total = 200;
        j.bytes_done = 50;
        assert_eq!(j.progress_percent(), 25.0);
        j.bytes_done = 500;
        assert_eq!(j.progress_percent(), 100.0);
    }
}
