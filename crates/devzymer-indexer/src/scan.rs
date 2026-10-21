use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use serde::Serialize;

use devzymer_core::model::Language;
use devzymer_core::util::relative_slash;
use devzymer_core::Result;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub exclude: Vec<String>,
    /// Excludes the files that carry credentials by convention.
    pub exclude_secret_files: bool,
    pub max_file_size: u64,
    pub follow_symlinks: bool,
    pub respect_gitignore: bool,
    pub include_hidden: bool,
    pub languages: Vec<Language>,
    pub max_workers: usize,
    /// Wall clock budget for the walk. When exceeded the walk stops early and
    /// the report is marked as truncated.
    pub max_duration_ms: Option<u64>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            exclude_secret_files: true,
            max_file_size: 2_000_000,
            follow_symlinks: false,
            respect_gitignore: true,
            include_hidden: false,
            languages: Vec::new(),
            max_workers: 0,
            max_duration_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScannedFile {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub size: u64,
    pub mtime_ms: u64,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    SecretContent,
    GitIgnored,
    Excluded,
    TooLarge,
    Binary,
    SymlinkEscape,
    Unreadable,
    LanguageFiltered,
    NotAFile,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedFile {
    pub rel_path: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub files: Vec<ScannedFile>,
    pub skipped: Vec<SkippedFile>,
    pub dirs_visited: u64,
    pub duration_ms: u64,
    /// True when the walk stopped before covering the whole tree.
    pub truncated: bool,
}

/// Names that are never useful as source and would only waste indexing time.
const ALWAYS_SKIPPED: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    ".desktop",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "composer.lock",
    "go.sum",
];

pub fn scan(root: &Path, options: &ScanOptions) -> Result<ScanReport> {
    let started = Instant::now();
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!options.include_hidden)
        .git_ignore(options.respect_gitignore)
        .git_global(options.respect_gitignore)
        .git_exclude(options.respect_gitignore)
        .ignore(true)
        .parents(options.respect_gitignore)
        .follow_links(options.follow_symlinks)
        .max_filesize(Some(options.max_file_size))
        .add_custom_ignore_filename(".devzymerignore");

    if options.max_workers > 0 {
        builder.threads(options.max_workers);
    }

    let mut override_builder = OverrideBuilder::new(root);
    if options.exclude_secret_files {
        for pattern in devzymer_core::privacy::SECRET_FILE_PATTERNS {
            let _ = override_builder.add(&format!("!{pattern}"));
        }
    }
    for pattern in &options.exclude {
        // The override builder follows gitignore semantics with the meaning of
        // `!` inverted: a plain glob whitelists, a glob prefixed with `!`
        // excludes. Every configured exclusion is therefore prefixed.
        let pattern = if pattern.starts_with('!') {
            pattern.clone()
        } else {
            format!("!{pattern}")
        };
        let _ = override_builder.add(&pattern);
    }
    if let Ok(overrides) = override_builder.build() {
        builder.overrides(overrides);
    }

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let files = std::sync::Arc::new(Mutex::new(Vec::<ScannedFile>::new()));
    let skipped = std::sync::Arc::new(Mutex::new(Vec::<SkippedFile>::new()));
    let dirs = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let truncated_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let root_owned = root.to_path_buf();

    let worker_files = files.clone();
    let worker_skipped = skipped.clone();
    let worker_dirs = dirs.clone();
    let worker_truncated = truncated_flag.clone();
    let worker_root = root_owned.clone();
    let worker_canonical_root = canonical_root.clone();
    let worker_options = options.clone();
    let deadline = options
        .max_duration_ms
        .map(|ms| started.elapsed().as_millis() as u64 + ms);

    builder.build_parallel().run(move || {
        let files = worker_files.clone();
        let skipped = worker_skipped.clone();
        let dirs = worker_dirs.clone();
        let root_owned = worker_root.clone();
        let canonical_root = worker_canonical_root.clone();
        let options = worker_options.clone();
        let truncated = worker_truncated.clone();
        Box::new(move |entry| {
            if let Some(deadline) = deadline {
                if started.elapsed().as_millis() as u64 > deadline {
                    truncated.store(true, std::sync::atomic::Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }
            }
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    if let Ok(mut guard) = skipped.lock() {
                        guard.push(SkippedFile {
                            rel_path: err.to_string(),
                            reason: SkipReason::Unreadable,
                        });
                    }
                    return ignore::WalkState::Continue;
                }
            };
            let path = entry.path();
            let rel =
                relative_slash(&root_owned, path).unwrap_or_else(|| path.display().to_string());

            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                dirs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return ignore::WalkState::Continue;
            }
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                push_skipped(&skipped, rel, SkipReason::NotAFile);
                return ignore::WalkState::Continue;
            }

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if ALWAYS_SKIPPED.contains(&name.as_str()) {
                push_skipped(&skipped, rel, SkipReason::Excluded);
                return ignore::WalkState::Continue;
            }

            if options.follow_symlinks {
                if let Ok(canonical) = path.canonicalize() {
                    if !canonical.starts_with(&canonical_root) {
                        push_skipped(&skipped, rel, SkipReason::SymlinkEscape);
                        return ignore::WalkState::Continue;
                    }
                }
            }

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => {
                    push_skipped(&skipped, rel, SkipReason::Unreadable);
                    return ignore::WalkState::Continue;
                }
            };
            let size = metadata.len();
            if size > options.max_file_size {
                push_skipped(&skipped, rel, SkipReason::TooLarge);
                return ignore::WalkState::Continue;
            }

            let language = Language::from_path(path);
            if !options.languages.is_empty() && !options.languages.contains(&language) {
                push_skipped(&skipped, rel, SkipReason::LanguageFiltered);
                return ignore::WalkState::Continue;
            }

            if is_binary(path) {
                push_skipped(&skipped, rel, SkipReason::Binary);
                return ignore::WalkState::Continue;
            }

            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            if let Ok(mut guard) = files.lock() {
                guard.push(ScannedFile {
                    rel_path: rel,
                    abs_path: path.to_path_buf(),
                    size,
                    mtime_ms,
                    language,
                });
            }
            ignore::WalkState::Continue
        })
    });

    let mut files_out = files.lock().map(|g| g.clone()).unwrap_or_default();
    let mut skipped_out = skipped.lock().map(|g| g.clone()).unwrap_or_default();
    files_out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    skipped_out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok(ScanReport {
        files: files_out,
        skipped: skipped_out,
        dirs_visited: dirs.load(std::sync::atomic::Ordering::Relaxed),
        duration_ms: started.elapsed().as_millis() as u64,
        truncated: truncated_flag.load(std::sync::atomic::Ordering::Relaxed),
    })
}

fn push_skipped(skipped: &Mutex<Vec<SkippedFile>>, rel_path: String, reason: SkipReason) {
    if let Ok(mut guard) = skipped.lock() {
        guard.push(SkippedFile { rel_path, reason });
    }
}

/// Detects binary content by looking for a NUL byte in the first block.
pub fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    let mut buf = [0u8; 8192];
    let Ok(n) = file.read(&mut buf) else {
        return true;
    };
    buf[..n].contains(&0)
}

/// Reads a file as UTF-8, replacing invalid sequences.
pub fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| devzymer_core::Error::io(path, e))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// BLAKE3 content hash used for change detection.
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_visits_the_workspace() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .to_path_buf();
        let report = scan(&root, &ScanOptions::default()).expect("scan");
        assert!(
            !report.files.is_empty(),
            "the scanner must find the workspace sources"
        );
        assert!(report.files.iter().any(|f| f.rel_path.ends_with("scan.rs")));
    }

    #[test]
    fn scan_with_default_exclusions() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .to_path_buf();
        let options = ScanOptions {
            exclude: devzymer_core::config::Config::default().index.exclude,
            ..ScanOptions::default()
        };
        let report = scan(&root, &options).expect("scan");
        assert!(
            !report.files.is_empty(),
            "the default exclusion list must not remove every source file"
        );
    }

    #[test]
    fn binary_files_are_detected() {
        let dir = std::env::temp_dir().join("devzymer-binary-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.bin");
        std::fs::write(&path, [0u8, 1, 2, 3, 0, 9]).unwrap();
        assert!(is_binary(&path));
        let text = dir.join("sample.txt");
        std::fs::write(&text, "conteudo de texto").unwrap();
        assert!(!is_binary(&text));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_hash_changes_with_content() {
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
        assert_eq!(content_hash(b"a"), content_hash(b"a"));
    }
}
