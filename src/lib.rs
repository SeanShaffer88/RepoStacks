//! RepoStacks core — every capability is an operation in this library first;
//! the CLI (and later the Tauri window) are thin surfaces over it.

pub mod atom;
pub mod cache_policy;
pub mod catalog;
pub mod glob_match;
pub mod http;
pub mod json;
pub mod net;
pub mod queue;
pub mod repo_ref;
pub mod search;
pub mod view;

pub use atom::{parse_releases_atom, ReleaseNote};
pub use cache_policy::{Auth, CachePolicy};
pub use catalog::{Catalog, RepoEntry};
pub use queue::{DownloadJob, JobStatus, Queue};
pub use repo_ref::RepoRef;
