//! Internal, patched hf-hub 1.0.0 client used by the XetGet CLI.
//!
//! Keep the upstream source and its patches unchanged. This target compiles the
//! source as part of the published xetget package, without a second crate release.
//! It does not provide a stable public library API.

// Upstream has its own formatting and lint conventions. Skip formatting of the
// imported modules so the source/patch parity check continues to work.
#![allow(clippy::all)]
// Upstream computes a Unix-only symlink path on Windows as well.
#![cfg_attr(windows, allow(unused_variables, unused_imports))]

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/client.rs"]
mod client;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/constants.rs"]
mod constants;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/error.rs"]
mod error;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/pagination.rs"]
mod pagination;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/retry.rs"]
mod retry;

#[cfg(feature = "blocking")]
#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/blocking.rs"]
mod blocking;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/buckets/mod.rs"]
pub mod buckets;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/cache/mod.rs"]
pub mod cache;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/progress.rs"]
pub mod progress;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/repository/mod.rs"]
pub mod repository;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/spaces.rs"]
pub mod spaces;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/users.rs"]
pub mod users;

#[rustfmt::skip]
#[path = "../vendor/hf-hub/src/xet.rs"]
mod xet;

#[cfg(feature = "blocking")]
pub use blocking::{HFBucketSync, HFClientSync, HFRepositorySync};
pub use buckets::HFBucket;
pub use client::{HFClient, HFClientBuilder, split_id};
#[doc(hidden)]
pub use constants::{hf_home, resolve_cache_dir};
pub use error::{HFError, HFResult, XetOperation};
pub use repository::{
    HFRepository, RepoType, RepoTypeAny, RepoTypeDataset, RepoTypeKernel, RepoTypeModel,
    RepoTypeSpace,
};
