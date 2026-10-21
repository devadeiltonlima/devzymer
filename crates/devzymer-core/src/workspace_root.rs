//! Workspace identity and fail-closed resolution.
//!
//! A workspace is the directory an index belongs to. Resolution never
//! escalates to an ancestor: an explicit root is taken literally, and
//! discovery stops at the user home, which is never a valid workspace.
//!
//! Every consumer binds to the resolved identity, so a server started from an
//! unexpected directory fails instead of serving another repository.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::util::canonical_display;
use crate::Result;

/// The files that identify a project root.
pub const ROOT_MARKERS: &[&str] = &[
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

/// How the workspace was identified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceKind {
    /// A git repository, with the directory that holds the shared git data.
    Git { common_dir: PathBuf },
    /// A directory identified only by its markers.
    Plain,
}

/// A validated workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceIdentity {
    /// The canonical absolute path of the workspace.
    pub root: PathBuf,
    /// A stable identifier derived from the canonical root.
    pub id: String,
    pub kind: WorkspaceKind,
    /// The markers that identified the workspace.
    pub markers: Vec<String>,
    /// True when the workspace is the user home or a filesystem root.
    pub broad: bool,
}

impl WorkspaceIdentity {
    /// The index that belongs to this workspace.
    pub fn index_path(&self) -> PathBuf {
        crate::paths::db_path(&self.root)
    }

    /// True when the workspace already holds an index.
    pub fn has_index(&self) -> bool {
        self.index_path().exists()
    }

    /// True when the workspace was identified by a git repository.
    pub fn is_git(&self) -> bool {
        matches!(self.kind, WorkspaceKind::Git { .. })
    }
}

/// What the caller is willing to accept.
#[derive(Debug, Clone, Copy, Default)]
pub struct Policy {
    /// Allows a workspace that is the user home or a filesystem root.
    pub allow_broad: bool,
    /// Allows a directory without project markers, when the path was given
    /// explicitly. This is the path that supports unversioned projects.
    pub allow_unversioned: bool,
}

impl Policy {
    /// The policy used when indexing: the caller asked for this directory.
    pub fn explicit() -> Self {
        Policy {
            allow_broad: true,
            allow_unversioned: true,
        }
    }
}

/// The markers present in a directory.
pub fn markers_found(dir: &Path) -> Vec<String> {
    ROOT_MARKERS
        .iter()
        .filter(|marker| dir.join(marker).exists())
        .map(|marker| marker.to_string())
        .collect()
}

/// True when the directory looks like the root of a project.
pub fn is_project_root(dir: &Path) -> bool {
    !markers_found(dir).is_empty()
}

/// The canonical path of a directory, or an error when it cannot be resolved.
pub fn canonical(path: &Path) -> Result<PathBuf> {
    let resolved = path.canonicalize().map_err(|e| {
        Error::WorkspaceNotResolved(format!(
            "nao foi possivel canonicalizar {}: {e}",
            path.display()
        ))
    })?;
    Ok(canonical_display(&resolved))
}

/// The canonical user home, when it exists.
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
        .and_then(|home| home.canonicalize().ok())
        .map(|home| canonical_display(&home))
}

/// True when the path is the user home, an ancestor of it, or a filesystem
/// root. Those directories hold unrelated personal data and are never valid
/// workspaces.
pub fn is_broad(root: &Path) -> bool {
    if let Some(home) = home_dir() {
        let home = canonical_display(&home);
        if root == home || home.starts_with(root) {
            return true;
        }
    }
    root.parent().is_none()
}

/// A stable identifier for a workspace, derived from its canonical path.
pub fn workspace_id(root: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(root.to_string_lossy().as_bytes());
    hasher.update(&[0u8]);
    hasher.update(
        std::fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .to_string_lossy()
            .as_bytes(),
    );
    hasher.finalize().to_hex()[..16].to_string()
}

/// Resolves an explicit path. The directory is taken literally: it is never
/// replaced by an ancestor.
pub fn resolve_explicit(path: &Path, policy: Policy) -> Result<WorkspaceIdentity> {
    let root = canonical(path)?;
    if !root.is_dir() {
        return Err(Error::WorkspaceNotResolved(format!(
            "{} nao e um diretorio",
            root.display()
        )));
    }
    if is_broad(&root) && !policy.allow_broad {
        return Err(Error::WorkspaceTooBroad(format!(
            "{} e o diretorio do usuario ou um ancestral dele; indexar esse \
             caminho coleta arquivos nao relacionados",
            root.display()
        )));
    }
    let markers = markers_found(&root);
    if markers.is_empty() && !policy.allow_unversioned {
        return Err(Error::WorkspaceNotResolved(format!(
            "{} nao tem marcador de projeto ({}); use --root para indexar \
             esse caminho explicitamente",
            root.display(),
            ROOT_MARKERS.join(", ")
        )));
    }
    Ok(build_identity(root, markers))
}

/// Finds the workspace that contains a directory.
///
/// The search goes up from the given directory and stops at the user home,
/// which is never returned. When nothing identifies a workspace, the caller
/// gets an error instead of an unrelated directory.
pub fn discover(start: &Path, policy: Policy) -> Result<WorkspaceIdentity> {
    let start = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| start.to_path_buf())
    };
    let start = match start.canonicalize() {
        Ok(resolved) => canonical_display(&resolved),
        Err(_) => start.clone(),
    };
    let home = home_dir().map(|home| canonical_display(&home));

    let mut current = Some(start.as_path());
    while let Some(dir) = current {
        if let Some(home) = home.as_ref() {
            if dir == home {
                break;
            }
            if home.starts_with(dir) {
                break;
            }
        }
        let markers = markers_found(dir);
        if !markers.is_empty() {
            let root = canonical_display(dir);
            if is_broad(&root) && !policy.allow_broad {
                break;
            }
            return Ok(build_identity(root, markers));
        }
        current = dir.parent();
    }

    Err(Error::WorkspaceNotResolved(format!(
        "nenhum workspace resolvido a partir de {}; use --root <diretorio>",
        start.display()
    )))
}

/// Resolves the workspace for a command.
///
/// An explicit path is authoritative. Without one, the workspace is
/// discovered from the working directory.
pub fn resolve(explicit: Option<&Path>, cwd: &Path, policy: Policy) -> Result<WorkspaceIdentity> {
    match explicit {
        Some(path) => resolve_explicit(path, policy),
        None => discover(cwd, policy),
    }
}

fn build_identity(root: PathBuf, markers: Vec<String>) -> WorkspaceIdentity {
    let kind = detect_git(&root);
    WorkspaceIdentity {
        id: workspace_id(&root),
        broad: is_broad(&root),
        root,
        kind,
        markers,
    }
}

/// Detects the git data of a directory without spawning a process.
fn detect_git(root: &Path) -> WorkspaceKind {
    let git_path = root.join(".git");
    if !git_path.exists() {
        return WorkspaceKind::Plain;
    }
    if git_path.is_dir() {
        let common = std::fs::read_to_string(git_path.join("commondir"))
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .map(|text| {
                let candidate = PathBuf::from(&text);
                let joined = if candidate.is_absolute() {
                    candidate
                } else {
                    root.join(candidate)
                };
                canonical_display(&joined)
            })
            .unwrap_or_else(|| root.join(".git"));
        return WorkspaceKind::Git { common_dir: common };
    }
    if let Ok(text) = std::fs::read_to_string(&git_path) {
        if let Some(rest) = text.trim().strip_prefix("gitdir: ") {
            let gitdir = PathBuf::from(rest);
            if let Some(parent) = gitdir.parent() {
                let joined = if parent.is_absolute() {
                    parent.to_path_buf()
                } else {
                    root.join(parent)
                };
                return WorkspaceKind::Git {
                    common_dir: canonical_display(&joined),
                };
            }
        }
    }
    WorkspaceKind::Git {
        common_dir: root.join(".git"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-ws-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn is_inside(child: &Path, parent: &Path) -> bool {
        child.starts_with(parent)
    }

    #[test]
    fn an_explicit_root_is_never_replaced_by_an_ancestor() {
        let root = temp_dir("explicit");
        let child = root.join("nested");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();

        let resolved = resolve_explicit(&child, Policy::explicit()).unwrap();
        assert!(
            is_inside(&resolved.root, &child),
            "a raiz explicita deve ser preservada"
        );
        assert_eq!(resolved.root, canonical_display(&child));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_stops_at_the_user_home() {
        let home = home_dir().expect("home");
        let outcome = discover(&home, Policy::default());
        assert!(
            outcome.is_err(),
            "a home do usuario nunca pode ser um workspace: {outcome:?}"
        );
        let _ = std::fs::remove_dir_all(home.join(".devzymer-ws-probe"));
    }

    #[test]
    fn discovery_finds_the_project_above_a_subdirectory() {
        let root = temp_dir("discover");
        let child = root.join("src").join("deep");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();

        let resolved = discover(&child, Policy::default()).unwrap();
        assert_eq!(resolved.root, canonical_display(&root));
        assert!(resolved.markers.iter().any(|m| m == "Cargo.toml"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_without_markers_is_rejected_unless_authorized() {
        let root = temp_dir("unversioned");
        let denied = resolve_explicit(&root, Policy::default());
        assert!(
            denied.is_err(),
            "sem marcador e sem autorizacao deve falhar"
        );

        let allowed = resolve_explicit(&root, Policy::explicit()).unwrap();
        assert_eq!(allowed.root, canonical_display(&root));
        assert!(allowed.markers.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_root_is_an_error_and_never_falls_back() {
        let root = temp_dir("missing");
        let target = root.join("nao-existe");
        let outcome = resolve_explicit(&target, Policy::explicit());
        assert!(outcome.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_home_directory_is_refused_as_a_broad_workspace() {
        let Some(home) = home_dir() else {
            return;
        };
        assert!(is_broad(&home));
        let denied = resolve_explicit(&home, Policy::default());
        assert!(denied.is_err(), "a home exige autorizacao explicita");
    }

    #[test]
    fn the_identity_is_stable_and_distinct() {
        let a = temp_dir("id-a");
        let b = temp_dir("id-b");
        let id_a = workspace_id(&canonical_display(&a));
        let id_b = workspace_id(&canonical_display(&b));
        assert_eq!(id_a, workspace_id(&canonical_display(&a)));
        assert_ne!(id_a, id_b);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
