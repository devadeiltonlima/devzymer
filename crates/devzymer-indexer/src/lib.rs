//! Repository scanning and structural analysis for devzymer.

pub mod fallback;
pub mod index;
pub mod parse;
pub mod scan;
pub mod sync;
pub mod watch;

pub use index::{IndexOptions, IndexReport, Indexer};
pub use parse::{extract_symbols, ParserRegistry};
pub use scan::{ScanOptions, ScanReport, ScannedFile, SkipReason, SkippedFile};
pub use sync::{reconcile, reindex_paths, SyncReport};
pub use watch::{WatchState, Watcher};
