//! The acquisitions queue: download jobs with pause/resume state.
//! The engine that moves bytes lands with the network slice; this module is
//! the state machine every surface (CLI, window, scheduler) drives.

use crate::repo_ref::RepoRef;

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

    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.is_active()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
