pub mod classify;
pub mod health;
pub mod host;
pub mod policy;
pub mod prompt;
pub mod session;
pub mod signals;
pub mod telemetry;

use std::path::{Path, PathBuf};

pub use devzymer_core::config::RoutingMode;

pub fn routing_dir(root: &Path) -> PathBuf {
    devzymer_core::paths::project_dir(root).join("routing")
}
