use std::path::{Path, PathBuf};

pub const PROJECT_DIR: &str = ".devzymer";
pub const DB_FILE: &str = "index.sqlite3";
pub const CONFIG_FILE: &str = "config.toml";

/// Root of the per-user devzymer state directory.
pub fn user_home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Directory holding the user level configuration file.
pub fn user_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| user_home().join(".config"))
        .join("devzymer")
}

/// Directory holding the user level cache and logs.
pub fn user_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| user_home().join(".local").join("share"))
        .join("devzymer")
}

/// Per project state directory.
pub fn project_dir(root: &Path) -> PathBuf {
    root.join(PROJECT_DIR)
}

pub fn db_path(root: &Path) -> PathBuf {
    project_dir(root).join(DB_FILE)
}

pub fn project_config_path(root: &Path) -> PathBuf {
    project_dir(root).join(CONFIG_FILE)
}

pub fn user_config_path() -> PathBuf {
    user_config_dir().join(CONFIG_FILE)
}

pub fn backup_dir(root: &Path) -> PathBuf {
    project_dir(root).join("backups")
}

/// The files that identify a project root.
const ROOT_MARKERS: &[&str] = &[
    ".git",
    ".devzymer",
    ".hg",
    ".svn",
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "requirements.txt",
    "composer.json",
    "Gemfile",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "deno.json",
    "deno.jsonc",
    "mix.exs",
    "pubspec.yaml",
];

/// True when the directory looks like the root of a project.
pub fn is_project_root(dir: &Path) -> bool {
    ROOT_MARKERS.iter().any(|marker| dir.join(marker).exists())
}

/// Finds the root of the project that contains `start`.
///
/// The directory itself wins whenever it looks like a project. The search only
/// goes up when it does not, and it never reaches the user home, which is not
/// a valid workspace. When no workspace is found the start directory is
/// returned, so callers that need a hard failure use
/// [`crate::workspace_root::discover`].
pub fn find_workspace_root(start: &Path) -> PathBuf {
    if is_project_root(start) {
        return start.to_path_buf();
    }
    let home = dirs::home_dir().and_then(|home| home.canonicalize().ok());
    let mut current = start.parent();
    while let Some(dir) = current {
        if let Some(home) = home.as_ref() {
            if dir == home || home.starts_with(dir) {
                break;
            }
        }
        if is_project_root(dir) {
            return dir.to_path_buf();
        }
        current = dir.parent();
    }
    start.to_path_buf()
}

/// The project root Claude Code exposes to MCP servers and hooks.
pub fn claude_project_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// The project root exposed by any host, independent of the vendor.
///
/// Hosts that do not provide a vendor specific variable can set this one, and
/// the server reaches the same project.
pub fn host_project_dir() -> Option<PathBuf> {
    std::env::var_os("DEVZYMER_PROJECT_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Current working directory, falling back to the process root.
pub fn current_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
