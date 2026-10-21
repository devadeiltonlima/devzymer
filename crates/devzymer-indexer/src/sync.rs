use std::path::{Path, PathBuf};
use std::time::Instant;

use devzymer_core::model::Language;
use devzymer_core::store::{FileRecord, Store};
use devzymer_core::util::relative_slash;
use devzymer_core::Result;
use devzymer_graph::build::rebuild_file_relations;
use serde::Serialize;

use crate::scan::{self, ScanOptions};

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: usize,
    pub symbols: usize,
    pub errors: Vec<String>,
    /// Files left out of the index, with the reason.
    pub skipped: Vec<String>,
    pub duration_ms: u64,
    pub truncated: bool,
}

impl SyncReport {
    pub fn touched(&self) -> usize {
        self.added.len() + self.modified.len() + self.removed.len()
    }

    /// True when the synchronization changed the index.
    pub fn changed(&self) -> bool {
        self.touched() > 0
    }
}

/// True when a path is under one of the excluded directories.
fn is_excluded(path: &str, exclude: &[String]) -> bool {
    if exclude.is_empty() {
        return false;
    }
    let normalized = path.replace('\\', "/");
    normalized.split('/').any(|component| {
        exclude.iter().any(|pattern| {
            let pattern = pattern.trim_start_matches('!');
            let pattern = pattern.trim_end_matches('/');
            !pattern.is_empty() && component == pattern
        })
    })
}

/// Reindexes only the given files, without walking the repository again.
///
/// A file that disappeared is removed from the index. A file whose content
/// hash did not change keeps its symbols and only has its metadata refreshed.
pub fn reindex_paths(store: &Store, root: &Path, paths: &[PathBuf]) -> Result<SyncReport> {
    let started = Instant::now();
    let mut report = SyncReport::default();
    let mut touched: Vec<String> = Vec::new();

    for path in paths {
        let Some(rel) = relative_slash(root, path) else {
            continue;
        };
        if !path.exists() {
            if store.file_by_path(&rel)?.is_some() {
                store.delete_relations_for_file(&rel)?;
                store.delete_file(&rel)?;
                report.removed.push(rel.clone());
                touched.push(rel);
            }
            continue;
        }
        match index_one(store, path, &rel) {
            Ok(IndexOutcome::Added) => {
                report.added.push(rel.clone());
                touched.push(rel);
            }
            Ok(IndexOutcome::Modified) => {
                report.modified.push(rel.clone());
                touched.push(rel);
            }
            Ok(IndexOutcome::Unchanged) => report.unchanged += 1,
            Err(err) => report.errors.push(format!("{rel}: {err}")),
        }
    }

    for rel in &touched {
        if let Err(err) = rebuild_file_relations(store, rel) {
            report.errors.push(format!("{rel}: relações: {err}"));
        }
    }

    if !touched.is_empty() {
        let pruned = store.prune_dangling_relations().unwrap_or(0);
        if pruned > 0 {
            report.removed.push(format!("{pruned} arestas obsoletas"));
        }
        store.bump_revision()?;
    }

    report.symbols = touched.len();
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

enum IndexOutcome {
    Added,
    Modified,
    Unchanged,
}

fn index_one(store: &Store, path: &Path, rel: &str) -> Result<IndexOutcome> {
    let bytes = std::fs::read(path).map_err(|e| devzymer_core::Error::io(path, e))?;
    let hash = scan::content_hash(&bytes);
    let metadata = std::fs::metadata(path).map_err(|e| devzymer_core::Error::io(path, e))?;
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let size = metadata.len();
    let language = Language::from_path(path);

    match store.file_by_path(rel)? {
        Some(existing) if existing.content_hash == hash => {
            let record = FileRecord {
                id: existing.id,
                path: rel.to_string(),
                language,
                size_bytes: size,
                mtime_ms,
                content_hash: hash,
                symbol_count: existing.symbol_count,
                indexed_at_ms: devzymer_core::util::now_millis(),
            };
            store.upsert_file(&record)?;
            Ok(IndexOutcome::Unchanged)
        }
        Some(existing) => {
            let source = String::from_utf8_lossy(&bytes).into_owned();
            let symbols = crate::parse::extract_symbols(rel, language, &source);
            let record = FileRecord {
                id: existing.id,
                path: rel.to_string(),
                language,
                size_bytes: size,
                mtime_ms,
                content_hash: hash,
                symbol_count: symbols.len(),
                indexed_at_ms: devzymer_core::util::now_millis(),
            };
            let file_id = store.upsert_file(&record)?;
            store.replace_symbols(file_id, &symbols)?;
            Ok(IndexOutcome::Modified)
        }
        None => {
            let source = String::from_utf8_lossy(&bytes).into_owned();
            let symbols = crate::parse::extract_symbols(rel, language, &source);
            let record = FileRecord {
                id: 0,
                path: rel.to_string(),
                language,
                size_bytes: size,
                mtime_ms,
                content_hash: hash,
                symbol_count: symbols.len(),
                indexed_at_ms: devzymer_core::util::now_millis(),
            };
            let file_id = store.upsert_file(&record)?;
            store.replace_symbols(file_id, &symbols)?;
            Ok(IndexOutcome::Added)
        }
    }
}

/// Compares the index against a metadata only scan and reindexes what
/// changed. The scan is bounded by `options.max_duration_ms`.
pub fn reconcile(store: &Store, root: &Path, options: &ScanOptions) -> Result<SyncReport> {
    let started = Instant::now();

    // The index was built with a different parser: its symbol data can no
    // longer be trusted, so it is rebuilt completely.
    if store.parser_version_changed() {
        let indexer = crate::index::Indexer::new(store, root);
        let index_options = crate::index::IndexOptions {
            root: root.to_path_buf(),
            scan: options.clone(),
            force_full: true,
            chunk_size: 256,
        };
        let index_report = indexer.index(&index_options)?;
        let mut report = SyncReport {
            added: vec![format!(
                "{} arquivos reindexados",
                index_report.files_indexed
            )],
            removed: vec![format!("{} arquivos removidos", index_report.files_removed)],
            unchanged: index_report.files_unchanged,
            symbols: index_report.symbols,
            errors: index_report.errors,
            skipped: index_report
                .skipped
                .iter()
                .map(|s| format!("{} ({:?})", s.rel_path, s.reason))
                .collect(),
            duration_ms: index_report.duration_ms,
            ..Default::default()
        };
        report.duration_ms = started.elapsed().as_millis() as u64;
        return Ok(report);
    }
    let scan = scan::scan(root, options)?;
    let mut report = SyncReport {
        truncated: scan.truncated,
        duration_ms: scan.duration_ms,
        skipped: scan
            .skipped
            .iter()
            .map(|s| format!("{} ({:?})", s.rel_path, s.reason))
            .collect(),
        ..Default::default()
    };

    let indexed: std::collections::HashMap<String, FileRecord> = store
        .all_files()?
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();

    let mut to_reindex: Vec<PathBuf> = Vec::new();
    for file in &scan.files {
        match indexed.get(&file.rel_path) {
            Some(existing)
                if existing.mtime_ms == file.mtime_ms
                    && existing.size_bytes == file.size
                    && !existing.content_hash.is_empty() =>
            {
                report.unchanged += 1;
            }
            _ => to_reindex.push(file.abs_path.clone()),
        }
    }

    let seen: std::collections::HashSet<&str> =
        scan.files.iter().map(|f| f.rel_path.as_str()).collect();
    let skipped: std::collections::HashSet<&str> =
        scan.skipped.iter().map(|s| s.rel_path.as_str()).collect();
    let mut removed: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    for path in indexed.keys() {
        if seen.contains(path.as_str()) {
            continue;
        }
        if skipped.contains(path.as_str()) {
            // The file is now excluded by configuration or ignore rules.
            removed.push(path.clone());
            continue;
        }
        if is_excluded(path, &options.exclude) {
            // A directory excluded by configuration is not visited at all, so
            // its files never reach the skipped list.
            removed.push(path.clone());
            continue;
        }
        // A file that is still on disk but absent from the scan was only
        // missed, for example because it was being written. It stays in the
        // index.
        if root.join(path).exists() {
            kept.push(path.clone());
        } else {
            removed.push(path.clone());
        }
    }
    for path in &kept {
        report
            .skipped
            .push(format!("{path} (presente no disco, ausente na varredura)"));
    }

    let partial = reindex_paths(store, root, &to_reindex)?;
    report.added = partial.added;
    report.modified = partial.modified;
    report.symbols = partial.symbols;
    report.errors = partial.errors;
    for path in &removed {
        store.delete_relations_for_file(path)?;
        store.delete_file(path)?;
        report.removed.push(path.clone());
    }
    if !removed.is_empty() {
        store.bump_revision()?;
    }

    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}
