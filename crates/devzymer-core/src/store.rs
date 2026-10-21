use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::{
    Confidence, Entity, Language, Relation, RelationKind, Span, Symbol, SymbolId, SymbolKind,
};
use crate::{Error, Result};

const SCHEMA_VERSION: u32 = 7;

/// The version of the symbol extraction. When it changes, the index built
/// with the previous version is no longer trustworthy and is rebuilt.
pub const PARSER_VERSION: &str = "devzymer-parser-6";

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: i64,
    pub path: String,
    pub language: Language,
    pub size_bytes: u64,
    pub mtime_ms: u64,
    pub content_hash: String,
    pub symbol_count: usize,
    pub indexed_at_ms: u64,
}

impl FileRecord {
    /// Lowercase file name without its extension, used for name based lookup.
    pub fn stem(&self) -> String {
        stem_of(&self.path)
    }
}

/// Lowercase file name without its extension.
pub fn stem_of(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = match file.rfind('.') {
        Some(index) if index > 0 => &file[..index],
        _ => file,
    };
    stem.to_ascii_lowercase()
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IndexStats {
    pub files: u64,
    pub symbols: u64,
    pub relations: u64,
    pub indexed_bytes: u64,
    pub revision: u64,
    pub schema_version: u32,
    pub parser_version: String,
    pub created_at_ms: u64,
    pub last_index_ms: u64,
    pub languages: Vec<(String, u64)>,
}

pub struct Store {
    conn: Connection,
}

/// The size of the index, measured by the database itself.
#[derive(Debug, Clone, Copy)]
pub struct StorageMetrics {
    pub page_size: u64,
    pub page_count: u64,
    pub freelist_pages: u64,
}

impl StorageMetrics {
    /// The size of the file in bytes.
    pub fn total_bytes(&self) -> u64 {
        self.page_size * self.page_count
    }

    /// The size of the free space inside the file.
    pub fn free_bytes(&self) -> u64 {
        self.page_size * self.freelist_pages
    }
}

/// The result of a checkpoint attempt. A busy outcome means the checkpoint did
/// not complete, which is reported instead of being treated as success.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointOutcome {
    pub busy: i64,
}

impl CheckpointOutcome {
    /// True when the checkpoint completed without contention.
    pub fn completed(&self) -> bool {
        self.busy == 0
    }
}

/// The workspace that owns an index path, when the path follows the layout
/// `<workspace>/.devzymer/index.sqlite3`.
fn workspace_root_of(path: &Path) -> Option<std::path::PathBuf> {
    let project = path.parent()?;
    if project.file_name()? != crate::paths::PROJECT_DIR {
        return None;
    }
    project.parent().map(|root| root.to_path_buf())
}

/// Registers the SQLite functions used by the lexical index triggers.
fn register_sql_functions(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "dz_split",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let value: String = ctx.get::<Option<String>>(0)?.unwrap_or_default();
            Ok(crate::util::split_identifier(&value))
        },
    )
    .map_err(Error::Database)?;
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let fresh = !path.exists();
        let conn = Connection::open(path).map_err(Error::Database)?;
        if fresh {
            // The reclamation of free pages only takes effect when the setting
            // is in place before the first table is created.
            conn.pragma_update(None, "auto_vacuum", "INCREMENTAL").ok();
        }
        let store = Self::from_connection(conn)?;
        if let Some(root) = workspace_root_of(path) {
            store.adopt_or_verify_workspace(&root)?;
        }
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory().map_err(Error::Database)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "synchronous", "NORMAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON").ok();
        // The write ahead log is bounded, so a long lived server does not grow
        // an unbounded file next to the index.
        conn.pragma_update(None, "journal_size_limit", 64 * 1024 * 1024)
            .ok();
        conn.pragma_update(None, "wal_autocheckpoint", 1000).ok();
        conn.busy_timeout(std::time::Duration::from_secs(10)).ok();
        register_sql_functions(&conn)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// The identifier of the workspace this index belongs to.
    pub fn workspace_id(&self) -> Result<Option<String>> {
        self.get_meta("workspace_id")
    }

    /// Records the workspace of this index, or refuses an index that belongs
    /// to another workspace.
    ///
    /// The identity is adopted the first time an index is opened, because the
    /// index lives inside the workspace. After that, an index moved or copied
    /// to another project is refused instead of serving foreign symbols.
    pub fn adopt_or_verify_workspace(&self, root: &Path) -> Result<()> {
        let expected = crate::workspace_root::workspace_id(root);
        match self.get_meta("workspace_id")? {
            None => {
                self.set_meta("workspace_id", &expected)?;
                Ok(())
            }
            Some(stored) if stored == expected => Ok(()),
            Some(stored) => Err(Error::WorkspaceMismatch(format!(
                "o indice em {} pertence a outro workspace ({stored});                  rode `devzymer init --root {}` para reindexar",
                root.join(crate::paths::PROJECT_DIR).display(),
                root.display()
            ))),
        }
    }

    /// The size of the index and of its write ahead log.
    pub fn storage_metrics(&self) -> Result<StorageMetrics> {
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap_or(4096);
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        let freelist: i64 = self
            .conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap_or(0);
        Ok(StorageMetrics {
            page_size: page_size as u64,
            page_count: page_count as u64,
            freelist_pages: freelist as u64,
        })
    }

    /// Returns free pages to the file, one step at a time.
    ///
    /// The step is bounded, so the operation never holds a long write lock on
    /// an index that other processes are reading.
    pub fn incremental_vacuum(&self, pages: u32) -> Result<u32> {
        let freed = self
            .conn
            .query_row("PRAGMA incremental_vacuum(?1)", params![pages], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0);
        Ok(freed.max(0) as u32)
    }

    /// Runs a passive checkpoint, so the write ahead log does not grow without
    /// bound. The outcome is reported, never assumed.
    pub fn checkpoint(&self) -> Result<CheckpointOutcome> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| r.get(0))
            .unwrap_or(-1);
        Ok(CheckpointOutcome { busy })
    }

    fn migrate(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
            .map_err(Error::Database)?;
        let current: u32 = self
            .get_meta("schema_version")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if current < 1 {
            self.conn
                .execute_batch(SCHEMA_V1)
                .map_err(Error::Database)?;
        }
        if current < 2 {
            self.migrate_to_v2()?;
        }
        if current < 3 {
            self.migrate_to_v3()?;
        }
        if current < 4 {
            self.migrate_to_v4()?;
        }
        if current < 6 {
            self.rebuild_lexical_index()?;
        }
        if current < 7 {
            self.rebuild_lexical_index()?;
        }
        self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        if self.get_meta("parser_version")?.is_none() {
            self.set_meta("parser_version", PARSER_VERSION)?;
        }
        if self.get_meta("created_at_ms")?.is_none() {
            self.set_meta("created_at_ms", &crate::util::now_millis().to_string())?;
        }
        if self.get_meta("revision")?.is_none() {
            self.set_meta("revision", "1")?;
        }
        Ok(())
    }

    /// Adds the indexed file stem used for name based lookup, keeping every
    /// already indexed file.
    fn migrate_to_v2(&mut self) -> Result<()> {
        let mut columns = self
            .conn
            .prepare("PRAGMA table_info(files)")
            .map_err(Error::Database)?;
        let mut names = Vec::new();
        let mut rows = columns.query([]).map_err(Error::Database)?;
        while let Some(row) = rows.next().map_err(Error::Database)? {
            names.push(row.get::<_, String>(1).map_err(Error::Database)?);
        }
        if !names.iter().any(|n| n == "stem") {
            self.conn
                .execute_batch("ALTER TABLE files ADD COLUMN stem TEXT NOT NULL DEFAULT '';")
                .map_err(Error::Database)?;
            let files = self.all_files()?;
            let mut stmt = self
                .conn
                .prepare("UPDATE files SET stem = ?1 WHERE id = ?2")
                .map_err(Error::Database)?;
            for file in &files {
                stmt.execute(params![file.stem(), file.id])
                    .map_err(Error::Database)?;
            }
        }
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_files_stem ON files(stem);")
            .map_err(Error::Database)?;
        Ok(())
    }

    /// The parser version of the running binary.
    /// Rebuilds the lexical index without the symbol bodies. The body of a
    /// symbol is kept in the table, but indexing it made a broad lexical query
    /// scan every row of the repository.
    fn migrate_to_v3(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "DROP TRIGGER IF EXISTS symbols_ai;
                 DROP TRIGGER IF EXISTS symbols_ad;
                 DROP TRIGGER IF EXISTS symbols_au;
                 DROP TABLE IF EXISTS symbol_fts;",
            )
            .map_err(Error::Database)?;
        self.conn
            .execute_batch(SCHEMA_FTS_V3)
            .map_err(Error::Database)?;
        // The lexical index is external content: it has to be rebuilt from the
        // symbols that are already indexed. Without this, the delete triggers
        // would try to remove rows that were never inserted and the index
        // would be left inconsistent.
        self.conn
            .execute_batch("INSERT INTO symbol_fts(symbol_fts) VALUES('rebuild');")
            .map_err(Error::Database)?;
        Ok(())
    }

    /// Replaces the lexical index by the plain version that keeps the
    /// identifier split into its words, with the triggers recreated to match.
    /// The symbol rows are untouched: the derived fts5 table is rebuilt from
    /// them, so no source information is lost.
    fn rebuild_lexical_index(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "DROP TRIGGER IF EXISTS symbols_ai;
                 DROP TRIGGER IF EXISTS symbols_ad;
                 DROP TRIGGER IF EXISTS symbols_au;
                 DROP TABLE IF EXISTS symbol_fts;",
            )
            .map_err(Error::Database)?;
        self.conn
            .execute_batch(SCHEMA_FTS_V7)
            .map_err(Error::Database)?;
        self.conn
            .execute_batch(
                "INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc, name_words, body)
                 SELECT rowid, COALESCE(name,''), COALESCE(qualified_name,''), COALESCE(signature,''),
                        COALESCE(doc,''), dz_split(COALESCE(name,'')), COALESCE(body,'') FROM symbols;",
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    /// Indexes the lower case form of the symbol names. Without them, a name
    /// lookup scanned every symbol of the repository, and a query with several
    /// terms paid that cost once per term.
    fn migrate_to_v4(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_symbols_name_lower ON symbols(lower(name));
                 CREATE INDEX IF NOT EXISTS idx_symbols_qname_lower ON symbols(lower(qualified_name));",
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn parser_version(&self) -> String {
        PARSER_VERSION.to_string()
    }

    /// The parser version used to build the current index content.
    pub fn indexed_parser_version(&self) -> Option<String> {
        self.get_meta("indexed_parser_version").ok().flatten()
    }

    /// True when the index was built with a different parser and its symbol
    /// data can no longer be trusted.
    pub fn parser_version_changed(&self) -> bool {
        self.indexed_parser_version()
            .map(|version| version != PARSER_VERSION)
            .unwrap_or(true)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(Error::Database)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn revision(&self) -> Result<u64> {
        Ok(self
            .get_meta("revision")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(1))
    }

    pub fn bump_revision(&self) -> Result<u64> {
        let next = self.revision()? + 1;
        self.set_meta("revision", &next.to_string())?;
        Ok(next)
    }

    pub fn touch_indexed(&self) -> Result<()> {
        self.set_meta("last_index_ms", &crate::util::now_millis().to_string())
    }

    // ----- files -----

    pub fn upsert_file(&self, rec: &FileRecord) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO files(path, language, size_bytes, mtime_ms, content_hash, symbol_count, indexed_at_ms, stem)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(path) DO UPDATE SET
                   language = excluded.language,
                   size_bytes = excluded.size_bytes,
                   mtime_ms = excluded.mtime_ms,
                   content_hash = excluded.content_hash,
                   symbol_count = excluded.symbol_count,
                   indexed_at_ms = excluded.indexed_at_ms,
                   stem = excluded.stem",
                params![
                    rec.path,
                    rec.language.display(),
                    rec.size_bytes as i64,
                    rec.mtime_ms as i64,
                    rec.content_hash,
                    rec.symbol_count as i64,
                    rec.indexed_at_ms as i64,
                    rec.stem()
                ],
            )
            .map_err(Error::Database)?;
        let id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![rec.path],
                |r| r.get(0),
            )
            .map_err(Error::Database)?;
        Ok(id)
    }

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRecord>> {
        self.conn
            .query_row(
                &format!("{FILE_COLUMNS} WHERE path = ?1"),
                params![path],
                row_to_file,
            )
            .optional()
            .map_err(Error::Database)
    }

    /// Files whose name matches the given stem, used to resolve a query that
    /// names a file instead of a symbol.
    pub fn files_by_stem(&self, stem: &str) -> Result<Vec<FileRecord>> {
        let stem = stem.to_ascii_lowercase();
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{FILE_COLUMNS} WHERE stem = ?1 ORDER BY path LIMIT 50"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![stem], row_to_file)
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(Error::Database)?);
        }
        Ok(out)
    }

    /// Files whose path ends with the given suffix.
    pub fn files_by_path_suffix(&self, suffix: &str) -> Result<Vec<FileRecord>> {
        let pattern = format!("%{}", suffix);
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{FILE_COLUMNS} WHERE path LIKE ?1 ESCAPE '\\' ORDER BY path LIMIT 50"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![pattern], row_to_file)
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(Error::Database)?);
        }
        Ok(out)
    }

    pub fn all_files(&self) -> Result<Vec<FileRecord>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{FILE_COLUMNS} ORDER BY path"))
            .map_err(Error::Database)?;
        let rows = stmt.query_map([], row_to_file).map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(Error::Database)?);
        }
        Ok(out)
    }

    pub fn delete_file(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn file_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .map_err(Error::Database)?;
        Ok(n as u64)
    }

    // ----- symbols -----

    pub fn replace_symbols(&self, file_id: i64, symbols: &[Symbol]) -> Result<()> {
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])
            .map_err(Error::Database)?;
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO symbols(id, file_id, name, qualified_name, kind, container, signature, doc,
                                     start_byte, end_byte, start_line, end_line, exported, is_async, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )
            .map_err(Error::Database)?;
        for s in symbols {
            stmt.execute(params![
                s.id.as_str(),
                file_id,
                s.name,
                s.qualified_name,
                s.kind.display(),
                s.container,
                s.signature,
                s.doc,
                s.span.start_byte as i64,
                s.span.end_byte as i64,
                s.span.start_line as i64,
                s.span.end_line as i64,
                s.exported as i64,
                s.is_async as i64,
                s.body
            ])
            .map_err(Error::Database)?;
        }
        Ok(())
    }

    pub fn symbols_for_file(&self, file_id: i64) -> Result<Vec<Symbol>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SYMBOL_COLUMNS} WHERE file_id = ?1 ORDER BY start_byte"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![file_id], row_to_symbol)
            .map_err(Error::Database)?;
        collect_symbols(rows)
    }

    pub fn symbol_by_id(&self, id: &str) -> Result<Option<Symbol>> {
        self.conn
            .query_row(
                &format!("{SYMBOL_COLUMNS} WHERE s.id = ?1"),
                params![id],
                row_to_symbol,
            )
            .optional()
            .map_err(Error::Database)
    }

    pub fn symbols_by_name(&self, name: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SYMBOL_COLUMNS} WHERE lower(name) = lower(?1) LIMIT 200"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![name], row_to_symbol)
            .map_err(Error::Database)?;
        collect_symbols(rows)
    }

    pub fn symbols_by_qualified_name(&self, qname: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SYMBOL_COLUMNS} WHERE lower(qualified_name) = lower(?1) LIMIT 200"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![qname], row_to_symbol)
            .map_err(Error::Database)?;
        collect_symbols(rows)
    }

    /// Symbols whose name holds the word anywhere, not only at the start.
    /// The scan has no index behind it, so it is a last resort for the words
    /// that only the segments of a larger name carry, and it is bounded.
    pub fn symbols_by_name_contains(&self, needle: &str, limit: usize) -> Result<Vec<Symbol>> {
        let mut pattern = String::with_capacity(needle.len() + 16);
        pattern.push('%');
        for ch in needle.chars() {
            if matches!(ch, '%' | '_' | '\\') {
                pattern.push('\\');
            }
            pattern.push(ch);
        }
        pattern.push('%');
        let lower = pattern.to_lowercase();
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SYMBOL_COLUMNS} WHERE lower(name) LIKE ?1 ESCAPE '\\' ORDER BY length(name), name LIMIT ?2"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![lower, limit as i64], row_to_symbol)
            .map_err(Error::Database)?;
        collect_symbols(rows)
    }

    /// The symbols whose name starts with the given prefix. A range is used
    /// instead of a pattern, so the index is actually used.
    pub fn symbols_with_prefix(&self, prefix: &str) -> Result<Vec<Symbol>> {
        let lower = prefix.to_ascii_lowercase();
        let upper = format!("{lower}\u{10FFFF}");
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SYMBOL_COLUMNS} WHERE lower(name) >= ?1 AND lower(name) < ?2 ORDER BY name LIMIT 200"
            ))
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![lower, upper], row_to_symbol)
            .map_err(Error::Database)?;
        collect_symbols(rows)
    }

    pub fn all_symbols(&self) -> Result<Vec<Symbol>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{SYMBOL_COLUMNS} ORDER BY qualified_name"))
            .map_err(Error::Database)?;
        let rows = stmt.query_map([], row_to_symbol).map_err(Error::Database)?;
        collect_symbols(rows)
    }

    pub fn symbol_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .map_err(Error::Database)?;
        Ok(n as u64)
    }

    /// Full text search over symbol metadata and body. Returns the symbol id
    /// and the BM25 score (lower is better in SQLite, so the sign is flipped).
    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        self.fts_search_weighted(query, limit, [1.0, 1.0, 1.0, 1.0, 1.0, 1.0])
    }

    /// Full text search with per-column BM25 weights, in the order the FTS
    /// table declares its columns: name, qualified_name, signature, doc,
    /// name_words, body.
    pub fn fts_search_weighted(
        &self,
        query: &str,
        limit: usize,
        weights: [f64; 6],
    ) -> Result<Vec<(String, f64)>> {
        let match_query = build_match_query(query);
        if match_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, bm25(symbol_fts, ?3, ?4, ?5, ?6, ?7, ?8) AS score
                 FROM symbol_fts f JOIN symbols s ON s.rowid = f.rowid
                 WHERE symbol_fts MATCH ?1
                 ORDER BY score LIMIT ?2",
            )
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(
                params![
                    match_query,
                    limit as i64,
                    weights[0],
                    weights[1],
                    weights[2],
                    weights[3],
                    weights[4],
                    weights[5]
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
            )
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            let (id, score) = r.map_err(Error::Database)?;
            out.push((id, -score));
        }
        Ok(out)
    }

    /// Full text search over an explicit set of terms. The terms are the
    /// forms the query uses that really exist in the repository, so the
    /// search does not depend on a generic word that only exists in the
    /// question.
    pub fn fts_search_terms(
        &self,
        terms: &[String],
        limit: usize,
        weights: [f64; 6],
    ) -> Result<Vec<(String, f64)>> {
        let mut parts: Vec<String> = Vec::new();
        for term in terms {
            let cleaned: String = term
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if cleaned.len() < 2 || LEXICAL_STOPWORDS.contains(&cleaned.to_lowercase().as_str()) {
                continue;
            }
            let escaped = format!("{cleaned}*");
            if !parts.contains(&escaped) {
                parts.push(escaped);
            }
        }
        if parts.is_empty() {
            return Ok(Vec::new());
        }
        let match_query = parts.join(" OR ");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, bm25(symbol_fts, ?3, ?4, ?5, ?6, ?7, ?8) AS score
                 FROM symbol_fts f JOIN symbols s ON s.rowid = f.rowid
                 WHERE symbol_fts MATCH ?1
                 ORDER BY score LIMIT ?2",
            )
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map(
                params![
                    match_query,
                    limit as i64,
                    weights[0],
                    weights[1],
                    weights[2],
                    weights[3],
                    weights[4],
                    weights[5]
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
            )
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            let (id, score) = r.map_err(Error::Database)?;
            out.push((id, -score));
        }
        Ok(out)
    }

    /// Number of indexed symbols that contain each term. The count comes from
    /// the full text index, so a term that only exists in a few symbols is
    /// rare and carries more meaning than a term present in every file.
    pub fn term_document_frequencies(&self, terms: &[String]) -> Result<HashMap<String, usize>> {
        let mut out = HashMap::new();
        if terms.is_empty() {
            return Ok(out);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT count(*) FROM symbol_fts WHERE symbol_fts MATCH ?1")
            .map_err(Error::Database)?;
        for term in terms {
            let match_query = build_match_query(term);
            if match_query.is_empty() {
                out.insert(term.clone(), 0);
                continue;
            }
            let count: i64 = stmt
                .query_row([match_query], |r| r.get(0))
                .map_err(Error::Database)?;
            out.insert(term.clone(), count.max(0) as usize);
        }
        Ok(out)
    }

    // ----- relations -----

    pub fn insert_relations(&self, relations: &[Relation]) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare(
                "INSERT INTO relations(src, dst, kind, confidence, evidence, file_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(Error::Database)?;
        for r in relations {
            stmt.execute(params![
                r.src.key(),
                r.dst.key(),
                r.kind.display(),
                r.confidence.display(),
                r.evidence,
                r.file
            ])
            .map_err(Error::Database)?;
        }
        Ok(())
    }

    pub fn delete_relations_for_file(&self, file: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM relations WHERE file_path = ?1", params![file])
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn delete_relations_for_symbol(&self, symbol_id: &str) -> Result<()> {
        let key = Entity::Symbol(SymbolId(symbol_id.to_string())).key();
        self.conn
            .execute(
                "DELETE FROM relations WHERE src = ?1 OR dst = ?1",
                params![key],
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn relations_from(&self, src_key: &str) -> Result<Vec<Relation>> {
        self.relation_query("src = ?1", src_key)
    }

    pub fn relations_to(&self, dst_key: &str) -> Result<Vec<Relation>> {
        self.relation_query("dst = ?1", dst_key)
    }

    fn relation_query(&self, predicate: &str, key: &str) -> Result<Vec<Relation>> {
        let sql = format!(
            "SELECT src, dst, kind, confidence, evidence, file_path FROM relations WHERE {predicate} LIMIT 2000"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(Error::Database)?;
        let rows = stmt
            .query_map(params![key], row_to_relation)
            .map_err(Error::Database)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(Error::Database)?);
        }
        Ok(out)
    }

    pub fn relation_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM relations", [], |r| r.get(0))
            .map_err(Error::Database)?;
        Ok(n as u64)
    }

    pub fn clear_relations(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM relations", [])
            .map_err(Error::Database)?;
        Ok(())
    }

    /// Discards the retrieval cache when the resolver changed, so results
    /// produced by a previous algorithm are never served.
    pub fn ensure_resolver_version(&self, version: &str) -> Result<bool> {
        match self.get_meta("resolver_version")? {
            Some(current) if current == version => Ok(false),
            _ => {
                self.conn
                    .execute("DELETE FROM retrieval_cache", [])
                    .map_err(Error::Database)?;
                self.set_meta("resolver_version", version)?;
                Ok(true)
            }
        }
    }

    /// Removes edges that point to symbols that no longer exist, which
    /// happens when a symbol is renamed or removed.
    pub fn prune_dangling_relations(&self) -> Result<usize> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM relations
                 WHERE (src LIKE 's:%' AND src NOT IN (SELECT 's:' || id FROM symbols))
                    OR (dst LIKE 's:%' AND dst NOT IN (SELECT 's:' || id FROM symbols))",
                [],
            )
            .map_err(Error::Database)?;
        Ok(removed)
    }

    // ----- caches -----

    pub fn cache_get(&self, key: &str, revision: u64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT payload FROM retrieval_cache WHERE key = ?1 AND revision = ?2",
                params![key, revision as i64],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(Error::Database)
    }

    pub fn cache_put(&self, key: &str, revision: u64, payload: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO retrieval_cache(key, revision, payload, created_at_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET revision = excluded.revision, payload = excluded.payload, created_at_ms = excluded.created_at_ms",
                params![key, revision as i64, payload, crate::util::now_millis() as i64],
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn cache_clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM retrieval_cache", [])
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn decision_cache_get(&self, key: &str, now_ms: u64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT payload FROM decision_cache WHERE key = ?1 AND expires_at_ms > ?2",
                params![key, now_ms as i64],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(Error::Database)
    }

    pub fn decision_cache_put(&self, key: &str, payload: &str, expires_at_ms: u64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO decision_cache(key, payload, created_at_ms, expires_at_ms) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET payload = excluded.payload, created_at_ms = excluded.created_at_ms, expires_at_ms = excluded.expires_at_ms",
                params![key, payload, crate::util::now_millis() as i64, expires_at_ms as i64],
            )
            .map_err(Error::Database)?;
        Ok(())
    }

    pub fn decision_cache_clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM decision_cache", [])
            .map_err(Error::Database)?;
        Ok(())
    }

    // ----- transactions -----

    pub fn begin(&self) -> Result<()> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(Error::Database)
    }

    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT").map_err(Error::Database)
    }

    pub fn rollback(&self) -> Result<()> {
        self.conn.execute_batch("ROLLBACK").map_err(Error::Database)
    }

    // ----- stats -----

    pub fn stats(&self) -> Result<IndexStats> {
        let files = self.file_count()?;
        let symbols = self.symbol_count()?;
        let relations = self.relation_count()?;
        let bytes: i64 = self
            .conn
            .query_row("SELECT COALESCE(SUM(size_bytes), 0) FROM files", [], |r| {
                r.get(0)
            })
            .map_err(Error::Database)?;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT language, COUNT(*) FROM files GROUP BY language ORDER BY COUNT(*) DESC",
            )
            .map_err(Error::Database)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(Error::Database)?;
        let mut languages = Vec::new();
        for r in rows {
            let (lang, count) = r.map_err(Error::Database)?;
            languages.push((lang, count as u64));
        }
        Ok(IndexStats {
            files,
            symbols,
            relations,
            indexed_bytes: bytes as u64,
            revision: self.revision()?,
            schema_version: SCHEMA_VERSION,
            parser_version: self.parser_version(),
            created_at_ms: self
                .get_meta("created_at_ms")?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            last_index_ms: self
                .get_meta("last_index_ms")?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            languages,
        })
    }

    pub fn integrity_check(&self) -> Result<String> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(Error::Database)?;
        Ok(result)
    }

    pub fn fts_available(&self) -> bool {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='symbol_fts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0
    }
}

const FILE_COLUMNS: &str = "SELECT id, path, language, size_bytes, mtime_ms, content_hash, symbol_count, indexed_at_ms FROM files";

const SYMBOL_COLUMNS: &str =
    "SELECT s.id, s.file_id, f.path, s.name, s.qualified_name, s.kind, s.container,
    s.signature, s.doc, s.start_byte, s.end_byte, s.start_line, s.end_line, s.exported, s.is_async,
    s.body, f.language
    FROM symbols s JOIN files f ON f.id = s.file_id";

fn row_to_file(r: &Row<'_>) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        id: r.get(0)?,
        path: r.get(1)?,
        language: Language::from_display(&r.get::<_, String>(2)?).unwrap_or(Language::Unknown),
        size_bytes: r.get::<_, i64>(3)? as u64,
        mtime_ms: r.get::<_, i64>(4)? as u64,
        content_hash: r.get(5)?,
        symbol_count: r.get::<_, i64>(6)? as usize,
        indexed_at_ms: r.get::<_, i64>(7)? as u64,
    })
}

fn row_to_symbol(r: &Row<'_>) -> rusqlite::Result<Symbol> {
    let path: String = r.get(2)?;
    let language = Language::from_display(&r.get::<_, String>(16)?).unwrap_or(Language::Unknown);
    let qualified_name: String = r.get(4)?;
    Ok(Symbol {
        id: SymbolId(r.get(0)?),
        name: r.get(3)?,
        qualified_name,
        kind: SymbolKind::from_display(&r.get::<_, String>(5)?),
        container: r.get(6)?,
        signature: r.get(7)?,
        doc: r.get(8)?,
        path,
        language,
        span: Span::new(
            r.get::<_, i64>(9)? as usize,
            r.get::<_, i64>(10)? as usize,
            r.get::<_, i64>(11)? as u32,
            r.get::<_, i64>(12)? as u32,
        ),
        exported: r.get::<_, i64>(13)? != 0,
        is_async: r.get::<_, i64>(14)? != 0,
        body: r.get(15)?,
    })
}

fn row_to_relation(r: &Row<'_>) -> rusqlite::Result<Relation> {
    let src =
        Entity::from_key(&r.get::<_, String>(0)?).unwrap_or(Entity::External("unknown".into()));
    let dst =
        Entity::from_key(&r.get::<_, String>(1)?).unwrap_or(Entity::External("unknown".into()));
    Ok(Relation {
        src,
        dst,
        kind: RelationKind::from_display(&r.get::<_, String>(2)?),
        confidence: Confidence::from_display(&r.get::<_, String>(3)?),
        evidence: r.get(4)?,
        file: r.get(5)?,
    })
}

fn collect_symbols(rows: impl Iterator<Item = rusqlite::Result<Symbol>>) -> Result<Vec<Symbol>> {
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(Error::Database)?);
    }
    Ok(out)
}

/// Builds a safe FTS5 MATCH expression from free-form user text.
/// Palavras genericas que nao devem virar termo de OR no FTS: elas aparecem
/// em qualquer frase e o BM25 acaba premiando textos curtos.
const LEXICAL_STOPWORDS: [&str; 67] = [
    "da", "de", "do", "das", "dos", "em", "sem", "que", "para", "quando", "algum", "alguns",
    "alguma", "algumas", "antes", "depois", "desde", "onde", "qual", "quais", "quantos", "quantas",
    "todo", "toda", "todos", "todas", "ate", "apos", "tambem", "entao", "mais", "cada", "sobre",
    "entre", "como", "assim", "ainda", "the", "and", "for", "with", "from", "that", "this", "when",
    "then", "while", "before", "after", "during", "only", "never", "both", "any", "some", "each",
    "every", "into", "onto", "over", "under", "than", "them", "they", "which", "where", "how",
];

fn build_match_query(query: &str) -> String {
    let mut terms = Vec::new();
    let mut split = false;
    // A path is a path. Splitting it into words would turn a precise query
    // into a broad one, and the file stage already answers it exactly.
    let path_like = query.contains('/') || query.contains('\\') || has_code_extension(query);
    for raw in query.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.') {
        let term = raw.trim_matches('.');
        if term.len() < 2 {
            continue;
        }
        let escaped: String = term
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        for part in escaped.split_whitespace() {
            // An identifier written in snake case is split into its words.
            // The search index keeps a camel case name as a single token, so
            // the whole identifier would be a phrase that nothing matches.
            if path_like {
                terms.push(format!("{part}*"));
                continue;
            }
            for word in part.split('_') {
                if word.len() < 2 {
                    continue;
                }
                if word != part {
                    split = true;
                }
                terms.push(format!("{word}*"));
            }
            // A word written in camel case (`VoiceCall`) is also matched by
            // each of its segments, which the index keeps separately.
            if part.chars().any(|c| c.is_ascii_uppercase()) {
                for word in crate::util::split_identifier(part).split(' ') {
                    if word.len() >= 2 {
                        terms.push(format!("{word}*"));
                    }
                }
            }
        }
    }
    terms.dedup();
    if terms.len() <= 2 && !split {
        return terms.join(" AND ");
    }
    // With more terms an AND eliminates almost every document, so the terms
    // are combined with OR and the ranking is left to BM25. Only the most
    // specific terms are kept, to bound the work.
    terms.sort_by_key(|term| std::cmp::Reverse(term.len()));
    terms.truncate(8);
    if terms.len() > 4 {
        terms.retain(|term| {
            let bare = term.trim_end_matches('*');
            bare.len() >= 3 && !LEXICAL_STOPWORDS.contains(&bare)
        });
    }
    terms.join(" OR ")
}

/// True when the query ends with the extension of a source file.
fn has_code_extension(query: &str) -> bool {
    const EXTENSIONS: [&str; 12] = [
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rs", "go", "java", "cs", "rb",
    ];
    match query.rsplit_once('.') {
        Some((_, ext)) => EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

const SCHEMA_FTS_V3: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(
  name,
  qualified_name,
  signature,
  doc,
  content='symbols',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc)
  VALUES (new.rowid, new.name, new.qualified_name, new.signature, new.doc);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
  INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, doc)
  VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature, old.doc);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
  INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, doc)
  VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature, old.doc);
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc)
  VALUES (new.rowid, new.name, new.qualified_name, new.signature, new.doc);
END;
"#;

/// The lexical index keeps a column with the identifier split into its words,
/// so a camel case name (`useLeituraVoiceCall`) is searchable by any of its
/// words (`voice`) instead of only by its whole form, and it keeps the source
/// text so that a behavior written in the implementation is searchable. The
/// table is a plain fts5 table, not external content, because the word column
/// is derived from `name` and the closure pattern of external tables would
/// reject it.
const SCHEMA_FTS_V7: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(
  name,
  qualified_name,
  signature,
  doc,
  name_words,
  body,
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc, name_words, body)
  VALUES (new.rowid, COALESCE(new.name,''), COALESCE(new.qualified_name,''),
          COALESCE(new.signature,''), COALESCE(new.doc,''), dz_split(COALESCE(new.name,'')),
          COALESCE(new.body,''));
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
  DELETE FROM symbol_fts WHERE rowid = old.rowid;
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
  DELETE FROM symbol_fts WHERE rowid = old.rowid;
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc, name_words, body)
  VALUES (new.rowid, COALESCE(new.name,''), COALESCE(new.qualified_name,''),
          COALESCE(new.signature,''), COALESCE(new.doc,''), dz_split(COALESCE(new.name,'')),
          COALESCE(new.body,''));
END;
"#;

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  language TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  mtime_ms INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  symbol_count INTEGER NOT NULL DEFAULT 0,
  indexed_at_ms INTEGER NOT NULL,
  stem TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS symbols (
  id TEXT PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  qualified_name TEXT NOT NULL,
  kind TEXT NOT NULL,
  container TEXT,
  signature TEXT,
  doc TEXT,
  start_byte INTEGER NOT NULL,
  end_byte INTEGER NOT NULL,
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  exported INTEGER NOT NULL DEFAULT 0,
  is_async INTEGER NOT NULL DEFAULT 0,
  body TEXT
);

CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_qname ON symbols(qualified_name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);

CREATE TABLE IF NOT EXISTS relations (
  id INTEGER PRIMARY KEY,
  src TEXT NOT NULL,
  dst TEXT NOT NULL,
  kind TEXT NOT NULL,
  confidence TEXT NOT NULL,
  evidence TEXT,
  file_path TEXT
);

CREATE INDEX IF NOT EXISTS idx_relations_src ON relations(src);
CREATE INDEX IF NOT EXISTS idx_relations_dst ON relations(dst);
CREATE INDEX IF NOT EXISTS idx_relations_kind ON relations(kind);
CREATE INDEX IF NOT EXISTS idx_relations_file ON relations(file_path);

CREATE VIRTUAL TABLE IF NOT EXISTS symbol_fts USING fts5(
  name,
  qualified_name,
  signature,
  doc,
  content='symbols',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc)
  VALUES (new.rowid, new.name, new.qualified_name, new.signature, new.doc);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
  INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, doc)
  VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature, old.doc);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
  INSERT INTO symbol_fts(symbol_fts, rowid, name, qualified_name, signature, doc)
  VALUES ('delete', old.rowid, old.name, old.qualified_name, old.signature, old.doc);
  INSERT INTO symbol_fts(rowid, name, qualified_name, signature, doc)
  VALUES (new.rowid, new.name, new.qualified_name, new.signature, new.doc);
END;

CREATE TABLE IF NOT EXISTS retrieval_cache (
  key TEXT PRIMARY KEY,
  revision INTEGER NOT NULL,
  payload TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS decision_cache (
  key TEXT PRIMARY KEY,
  payload TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Language, Span, Symbol, SymbolId, SymbolKind};

    fn symbol(name: &str, path: &str) -> Symbol {
        Symbol {
            id: SymbolId::new(path, name),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            container: None,
            signature: Some(format!("fn {name}()")),
            doc: Some("documentação".into()),
            path: path.to_string(),
            language: Language::Rust,
            span: Span::new(0, 100, 1, 10),
            exported: true,
            is_async: false,
            body: Some(format!("fn {name}() {{ println!(\"oi\"); }}")),
        }
    }

    #[test]
    fn an_identifier_in_snake_case_is_searched_by_its_words() {
        let query = build_match_query("call_preparar_numero_de_teste");
        assert!(query.contains("preparar*"), "{query}");
        assert!(query.contains("teste*"), "{query}");
        assert!(
            !query.contains("call_preparar_numero_de_teste*"),
            "the whole identifier would be a phrase that nothing matches: {query}"
        );
    }

    #[test]
    fn a_single_name_keeps_its_exact_prefix() {
        let query = build_match_query("useVoiceCall");
        assert!(query.starts_with("useVoiceCall*"), "{query}");
        assert!(query.contains("voice*"), "{query}");
        assert_eq!(build_match_query("objeto"), "objeto*");
    }

    #[test]
    fn the_lexical_index_keeps_the_words_of_a_camel_case_name() {
        let store = Store::open_in_memory().unwrap();
        let record = FileRecord {
            id: 0,
            path: "src/hooks/useVoiceCall.ts".into(),
            language: Language::TypeScript,
            size_bytes: 120,
            mtime_ms: 1000,
            content_hash: "abc".into(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let mut sym = symbol("useLeituraVoiceCall", "src/hooks/useVoiceCall.ts");
        sym.language = Language::TypeScript;
        sym.doc = None;
        store.replace_symbols(file_id, &[sym]).unwrap();
        for term in ["voice*", "call*", "leitura*"] {
            let hits = store.fts_search(term, 10).unwrap();
            assert!(
                !hits.is_empty(),
                "{term} had no answer even though the name holds the word"
            );
        }
    }

    #[test]
    fn replacing_symbols_keeps_the_lexical_index_consistent() {
        let store = Store::open_in_memory().unwrap();
        let record = FileRecord {
            id: 0,
            path: "src/lib/api.ts".into(),
            language: Language::TypeScript,
            size_bytes: 40,
            mtime_ms: 1000,
            content_hash: "abc".into(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let mut first = symbol("tarefaAntiga", "src/lib/api.ts");
        first.language = Language::TypeScript;
        let mut second = symbol("tarefaAntiga", "src/lib/api.ts");
        second.language = Language::TypeScript;
        second.name = "tarefaNova".into();
        second.qualified_name = "tarefaNova".into();
        second.id = SymbolId::new("src/lib/api.ts", "tarefaNova");
        store.replace_symbols(file_id, &[first]).unwrap();
        store.replace_symbols(file_id, &[second]).unwrap();
        assert!(store.fts_search("antiga*", 10).unwrap().is_empty());
        assert_eq!(store.fts_search("nova*", 10).unwrap().len(), 1);
    }

    #[test]
    fn a_new_resolver_version_discards_the_retrieval_cache() {
        let store = Store::open_in_memory().unwrap();
        store.cache_put("chave", 1, "payload").unwrap();
        assert!(store.cache_get("chave", 1).unwrap().is_some());
        assert!(store.ensure_resolver_version("resolver-1").unwrap());
        assert!(store.cache_get("chave", 1).unwrap().is_none());
        store.cache_put("chave", 1, "payload").unwrap();
        assert!(!store.ensure_resolver_version("resolver-1").unwrap());
        assert!(store.cache_get("chave", 1).unwrap().is_some());
        assert!(store.ensure_resolver_version("resolver-2").unwrap());
        assert!(store.cache_get("chave", 1).unwrap().is_none());
    }

    #[test]
    fn files_and_symbols_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let record = FileRecord {
            id: 0,
            path: "src/lib.rs".into(),
            language: Language::Rust,
            size_bytes: 120,
            mtime_ms: 1000,
            content_hash: "abc".into(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        store
            .replace_symbols(file_id, &[symbol("validate_session", "src/lib.rs")])
            .unwrap();

        let found = store.symbols_by_name("validate_session").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "src/lib.rs");
        assert_eq!(found[0].span.start_line, 1);
        assert!(store.symbols_by_name("VALIDATE_SESSION").unwrap().len() == 1);
    }

    #[test]
    fn full_text_search_finds_symbol_bodies() {
        let store = Store::open_in_memory().unwrap();
        let record = FileRecord {
            id: 0,
            path: "src/lib.rs".into(),
            language: Language::Rust,
            size_bytes: 120,
            mtime_ms: 1000,
            content_hash: "abc".into(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        store
            .replace_symbols(file_id, &[symbol("validate_session", "src/lib.rs")])
            .unwrap();

        let hits = store.fts_search("validate_session", 10).unwrap();
        assert!(!hits.is_empty(), "the lexical index must return a match");
        let resolved = store.symbol_by_id(&hits[0].0).unwrap().unwrap();
        assert_eq!(resolved.name, "validate_session");
    }

    #[test]
    fn relations_are_stored_per_entity() {
        let store = Store::open_in_memory().unwrap();
        let relation = Relation {
            src: Entity::File("src/a.rs".into()),
            dst: Entity::Symbol(SymbolId::new("src/b.rs", "helper")),
            kind: RelationKind::Imports,
            confidence: Confidence::Confirmed,
            evidence: Some("crate::b".into()),
            file: Some("src/a.rs".into()),
        };
        store.insert_relations(&[relation]).unwrap();
        let outgoing = store
            .relations_from(&Entity::File("src/a.rs".into()).key())
            .unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].kind, RelationKind::Imports);
        let incoming = store
            .relations_to(&Entity::Symbol(SymbolId::new("src/b.rs", "helper")).key())
            .unwrap();
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn stats_report_the_index_contents() {
        let store = Store::open_in_memory().unwrap();
        let record = FileRecord {
            id: 0,
            path: "src/lib.rs".into(),
            language: Language::Rust,
            size_bytes: 120,
            mtime_ms: 1000,
            content_hash: "abc".into(),
            symbol_count: 1,
            indexed_at_ms: 2000,
        };
        let file_id = store.upsert_file(&record).unwrap();
        store
            .replace_symbols(file_id, &[symbol("validate_session", "src/lib.rs")])
            .unwrap();
        let stats = store.stats().unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.symbols, 1);
        assert!(stats.revision >= 1);
        assert!(store.integrity_check().unwrap() == "ok");
    }

    #[test]
    fn a_parser_version_change_is_detected() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store.parser_version_changed(),
            "an index without a parser version cannot be trusted"
        );
        store
            .set_meta("indexed_parser_version", crate::store::PARSER_VERSION)
            .unwrap();
        assert!(!store.parser_version_changed());
        store
            .set_meta("indexed_parser_version", "parser-antigo")
            .unwrap();
        assert!(store.parser_version_changed());
    }

    #[test]
    fn cache_entries_are_bound_to_the_revision() {
        let store = Store::open_in_memory().unwrap();
        let revision = store.revision().unwrap();
        store.cache_put("chave", revision, "valor").unwrap();
        assert_eq!(
            store.cache_get("chave", revision).unwrap().as_deref(),
            Some("valor")
        );
        assert!(store.cache_get("chave", revision + 1).unwrap().is_none());
    }
}
