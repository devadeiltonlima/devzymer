use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use devzymer_core::store::Store;
use devzymer_core::Result;
use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::scan::ScanOptions;
use crate::sync::{reindex_paths, SyncReport};

const DEBOUNCE: Duration = Duration::from_millis(700);

#[derive(Debug, Default)]
pub struct WatchState {
    pub running: AtomicBool,
    pub pending: AtomicU64,
    pub syncs: AtomicU64,
    pub files_changed: AtomicU64,
    pub last_sync_ms: AtomicU64,
    pub last_error: Mutex<Option<String>>,
    pub last_report: Mutex<Option<SyncReport>>,
    pub root: Mutex<Option<String>>,
}

impl WatchState {
    pub fn snapshot(&self) -> WatchSnapshot {
        WatchSnapshot {
            running: self.running.load(Ordering::Relaxed),
            pending: self.pending.load(Ordering::Relaxed),
            syncs: self.syncs.load(Ordering::Relaxed),
            files_changed: self.files_changed.load(Ordering::Relaxed),
            last_sync_ms: self.last_sync_ms.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|g| g.clone()),
            root: self.root.lock().ok().and_then(|g| g.clone()),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct WatchSnapshot {
    pub running: bool,
    pub pending: u64,
    pub syncs: u64,
    pub files_changed: u64,
    pub last_sync_ms: u64,
    pub last_error: Option<String>,
    pub root: Option<String>,
}

/// Keeps the index synchronized with the filesystem.
pub struct Watcher {
    /// Retained on purpose: dropping the watcher stops the notifications.
    #[allow(dead_code)]
    handle: Option<notify::RecommendedWatcher>,
    state: Arc<WatchState>,
}

impl Watcher {
    /// Starts watching the workspace. The watcher keeps its own database
    /// connection, so it never blocks a reader.
    pub fn start(root: &Path, db_path: &Path, scan: ScanOptions) -> Result<Watcher> {
        let state = Arc::new(WatchState::default());
        *state
            .root
            .lock()
            .map_err(|_| devzymer_core::Error::other("estado do watcher corrompido"))? =
            Some(root.display().to_string());

        let (tx, rx) = mpsc::channel::<PathBuf>();
        let mut handle =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                            for path in event.paths {
                                let _ = tx.send(path);
                            }
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?;
        handle
            .watch(root, RecursiveMode::Recursive)
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?;

        let worker_state = state.clone();
        let worker_root = root.to_path_buf();
        let worker_db = db_path.to_path_buf();
        let worker_scan = scan.clone();
        let thread_state = worker_state.clone();
        let worker = std::thread::Builder::new()
            .name("devzymer-watch".to_string())
            .spawn(move || {
                run(worker_root, worker_db, worker_scan, rx, thread_state);
            })
            .map_err(|e| devzymer_core::Error::other(e.to_string()))?;
        worker_state.running.store(true, Ordering::Relaxed);
        let _ = worker;

        Ok(Watcher {
            handle: Some(handle),
            state,
        })
    }

    pub fn state(&self) -> Arc<WatchState> {
        self.state.clone()
    }
}

fn run(
    root: PathBuf,
    db_path: PathBuf,
    scan: ScanOptions,
    rx: mpsc::Receiver<PathBuf>,
    state: Arc<WatchState>,
) {
    let store = match Store::open(&db_path) {
        Ok(store) => store,
        Err(err) => {
            if let Ok(mut guard) = state.last_error.lock() {
                *guard = Some(err.to_string());
            }
            state.running.store(false, Ordering::Relaxed);
            return;
        }
    };

    let mut dirty: HashSet<PathBuf> = HashSet::new();
    loop {
        match rx.recv_timeout(DEBOUNCE) {
            Ok(path) => {
                if is_interesting(&root, &path, &scan) {
                    dirty.insert(path);
                    state.pending.store(dirty.len() as u64, Ordering::Relaxed);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if !dirty.is_empty() {
                    flush(&store, &root, &scan, &mut dirty, &state);
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !dirty.is_empty() {
                    flush(&store, &root, &scan, &mut dirty, &state);
                }
                break;
            }
        }
    }
    state.running.store(false, Ordering::Relaxed);
}

fn flush(
    store: &Store,
    root: &Path,
    scan: &ScanOptions,
    dirty: &mut HashSet<PathBuf>,
    state: &Arc<WatchState>,
) {
    let paths: Vec<PathBuf> = dirty.drain().collect();
    state.pending.store(0, Ordering::Relaxed);
    let started = Instant::now();
    match reindex_paths(store, root, &paths) {
        Ok(report) => {
            state
                .files_changed
                .fetch_add(report.touched() as u64, Ordering::Relaxed);
            state.syncs.fetch_add(1, Ordering::Relaxed);
            state
                .last_sync_ms
                .store(devzymer_core::util::now_millis(), Ordering::Relaxed);
            if let Ok(mut guard) = state.last_report.lock() {
                *guard = Some(report);
            }
            if let Ok(mut guard) = state.last_error.lock() {
                *guard = None;
            }
        }
        Err(err) => {
            if let Ok(mut guard) = state.last_error.lock() {
                *guard = Some(err.to_string());
            }
        }
    }
    let _ = started;
    let _ = scan;
}

/// Ignores events for paths that the indexer never covers, so the watcher
/// does not drown in build output.
fn is_interesting(root: &Path, path: &Path, scan: &ScanOptions) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.starts_with(".devzymer/") || text == ".devzymer" {
        return false;
    }
    for component in text.split('/') {
        if component == ".git" || component == "node_modules" {
            return false;
        }
        if scan.exclude.iter().any(|pattern| {
            let pattern = pattern.trim_start_matches('!');
            !pattern.is_empty() && component == pattern
        }) {
            return false;
        }
    }
    true
}
