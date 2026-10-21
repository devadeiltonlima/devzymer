use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;
use serde::Serialize;

use devzymer_core::model::Symbol;
use devzymer_core::store::{FileRecord, Store};
use devzymer_core::Result;

use crate::scan::{self, ScanOptions, ScanReport, ScannedFile, SkipReason, SkippedFile};

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub root: PathBuf,
    pub scan: ScanOptions,
    pub force_full: bool,
    pub chunk_size: usize,
}

impl IndexOptions {
    pub fn new(root: impl Into<PathBuf>, scan: ScanOptions) -> Self {
        Self {
            root: root.into(),
            scan,
            force_full: false,
            chunk_size: 256,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct IndexReport {
    pub files_scanned: usize,
    pub files_indexed: usize,
    pub files_unchanged: usize,
    pub files_removed: usize,
    pub files_renamed: usize,
    pub files_skipped: usize,
    pub symbols: usize,
    pub duration_ms: u64,
    pub errors: Vec<String>,
    pub skipped: Vec<SkippedFile>,
}

pub struct Indexer<'a> {
    store: &'a Store,
}

impl<'a> Indexer<'a> {
    pub fn new(store: &'a Store, _root: impl Into<PathBuf>) -> Self {
        Self { store }
    }

    pub fn index(&self, options: &IndexOptions) -> Result<IndexReport> {
        let started = Instant::now();
        let report = self.scan(options)?;
        let existing = self.store.all_files()?;
        let existing_map: HashMap<String, FileRecord> =
            existing.into_iter().map(|f| (f.path.clone(), f)).collect();

        let scanned_map: HashMap<String, &ScannedFile> = report
            .files
            .iter()
            .map(|f| (f.rel_path.clone(), f))
            .collect();

        let mut to_parse: Vec<ScannedFile> = Vec::new();
        let mut unchanged = 0usize;

        if options.force_full {
            to_parse = report.files.clone();
        } else {
            for file in &report.files {
                match existing_map.get(&file.rel_path) {
                    Some(prev)
                        if prev.mtime_ms == file.mtime_ms
                            && prev.size_bytes == file.size
                            && !prev.content_hash.is_empty() =>
                    {
                        unchanged += 1;
                    }
                    _ => to_parse.push(file.clone()),
                }
            }
        }

        let removed: Vec<String> = existing_map
            .keys()
            .filter(|p| !scanned_map.contains_key(*p))
            .cloned()
            .collect();

        let hash_by_removed: HashMap<String, String> = existing_map
            .iter()
            .filter(|(_, f)| !f.content_hash.is_empty())
            .map(|(p, f)| (p.clone(), f.content_hash.clone()))
            .collect();

        let mut errors: Vec<String> = Vec::new();
        let mut symbols_written = 0usize;
        let mut indexed = 0usize;
        let mut new_hashes: HashMap<String, String> = HashMap::new();
        let mut skipped: Vec<SkippedFile> = Vec::new();

        self.store.begin()?;

        for chunk in to_parse.chunks(options.chunk_size.max(1)) {
            let results: Vec<Result<(ScannedFile, Vec<Symbol>, bool)>> =
                chunk.par_iter().map(Self::parse_file).collect();
            for result in results {
                match result {
                    Ok((file, symbols, secret)) => {
                        if secret {
                            skipped.push(SkippedFile {
                                rel_path: file.rel_path.clone(),
                                reason: SkipReason::SecretContent,
                            });
                            continue;
                        }
                        let hash = self.hash_of(&file)?;
                        let record = FileRecord {
                            id: 0,
                            path: file.rel_path.clone(),
                            language: file.language,
                            size_bytes: file.size,
                            mtime_ms: file.mtime_ms,
                            content_hash: hash.clone(),
                            symbol_count: symbols.len(),
                            indexed_at_ms: devzymer_core::util::now_millis(),
                        };
                        let file_id = self.store.upsert_file(&record)?;
                        self.store.replace_symbols(file_id, &symbols)?;
                        new_hashes.insert(file.rel_path.clone(), hash);
                        symbols_written += symbols.len();
                        indexed += 1;
                    }
                    Err(err) => errors.push(err.to_string()),
                }
            }
        }

        let mut renames = 0usize;
        for path in &removed {
            self.store.delete_relations_for_file(path)?;
            self.store.delete_file(path)?;
            if let Some(old_hash) = hash_by_removed.get(path) {
                if new_hashes.values().any(|h| h == old_hash) {
                    renames += 1;
                }
            }
        }

        let result = self.store.commit();
        if result.is_err() {
            let _ = self.store.rollback();
            return result.map(|_| IndexReport::default());
        }

        self.store.bump_revision()?;
        self.store.touch_indexed()?;
        if options.force_full {
            // Every symbol was produced by the current parser.
            self.store.set_meta(
                "indexed_parser_version",
                devzymer_core::store::PARSER_VERSION,
            )?;
        }

        Ok(IndexReport {
            files_scanned: report.files.len(),
            files_indexed: indexed,
            files_unchanged: unchanged,
            files_removed: removed.len(),
            files_renamed: renames,
            files_skipped: report.skipped.len(),
            symbols: symbols_written,
            duration_ms: started.elapsed().as_millis() as u64,
            errors,
            skipped: skipped.into_iter().chain(report.skipped).collect(),
        })
    }

    fn scan(&self, options: &IndexOptions) -> Result<ScanReport> {
        scan::scan(&options.root, &options.scan)
    }

    fn hash_of(&self, file: &ScannedFile) -> Result<String> {
        let bytes = std::fs::read(&file.abs_path)
            .map_err(|e| devzymer_core::Error::io(&file.abs_path, e))?;
        Ok(scan::content_hash(&bytes))
    }

    fn parse_file(file: &ScannedFile) -> Result<(ScannedFile, Vec<Symbol>, bool)> {
        let source = scan::read_text(&file.abs_path)?;
        if devzymer_core::privacy::looks_secret_content(&source) {
            return Ok((file.clone(), Vec::new(), true));
        }
        let symbols = crate::parse::extract_symbols(&file.rel_path, file.language, &source);
        Ok((file.clone(), symbols, false))
    }

    /// Detects files whose content changed since the last index run, without
    /// re-parsing them.
    pub fn changed_files(&self, options: &IndexOptions) -> Result<Vec<ScannedFile>> {
        let report = self.scan(options)?;
        let existing: HashMap<String, FileRecord> = self
            .store
            .all_files()?
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();
        Ok(report
            .files
            .into_iter()
            .filter(|f| match existing.get(&f.rel_path) {
                Some(prev) => {
                    prev.mtime_ms != f.mtime_ms
                        || prev.size_bytes != f.size
                        || prev.content_hash.is_empty()
                }
                None => true,
            })
            .collect())
    }
}

/// Removes index entries for files that no longer exist on disk.
pub fn prune_missing(store: &Store, root: &Path) -> Result<usize> {
    let existing = store.all_files()?;
    let mut removed = 0usize;
    for record in existing {
        let abs = root.join(&record.path);
        if !abs.exists() {
            store.delete_relations_for_file(&record.path)?;
            store.delete_file(&record.path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Summary of why files were skipped, useful for the status report.
pub fn skip_summary(skipped: &[SkippedFile]) -> Vec<(SkipReason, usize)> {
    let mut counts: HashMap<SkipReason, usize> = HashMap::new();
    for s in skipped {
        *counts.entry(s.reason).or_insert(0) += 1;
    }
    let mut out: Vec<(SkipReason, usize)> = counts.into_iter().collect();
    out.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    out
}
