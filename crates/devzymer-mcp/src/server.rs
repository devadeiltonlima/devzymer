use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use devzymer_core::config::Config;
use devzymer_core::store::Store;
use devzymer_core::Result;
use devzymer_decisions::router::build_remote_provider;
use devzymer_decisions::DecisionRouter;
use devzymer_indexer::scan::ScanOptions;
use devzymer_indexer::sync::{reconcile, reindex_paths, SyncReport};
use devzymer_indexer::watch::Watcher;
use devzymer_routing::health::{self, Heartbeat};
use devzymer_routing::host::Host;
use devzymer_routing::signals;
use devzymer_routing::telemetry::{self, kind, observed, Event};
use serde_json::{json, Value};

use crate::tools::dispatch;

pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

/// A block of source that was delivered to the caller.
#[derive(Debug, Clone)]
pub struct ReadRecord {
    pub start_line: u32,
    pub end_line: u32,
    pub sha: String,
    pub truncated: bool,
}

/// Freshness of the index, exposed by `context_status`.
#[derive(Debug, Default)]
pub struct ServerState {
    pub last_sync_ms: AtomicU64,
    pub last_reconcile_ms: AtomicU64,
    pub reconciling: AtomicBool,
    pub syncs: AtomicU64,
    pub failures: Mutex<Vec<String>>,
    pub last_report: Mutex<Option<SyncReport>>,
    /// The last excerpt delivered for each file. A patch may only address a
    /// block inside the excerpt the caller actually received.
    pub reads: Mutex<std::collections::HashMap<String, ReadRecord>>,
    pub client: Mutex<Host>,
}

pub struct Server {
    pub root: PathBuf,
    pub store: Store,
    pub config: Config,
    pub router: DecisionRouter,
    watcher: Option<Arc<Watcher>>,
    pub state: Arc<ServerState>,
}

impl Server {
    pub fn new(root: impl Into<PathBuf>, config: Config) -> Result<Server> {
        let root = root.into();
        let db_path = devzymer_core::paths::db_path(&root);
        if !db_path.exists() {
            return Err(devzymer_core::Error::WorkspaceNotResolved(format!(
                "nenhum indice devzymer em {}; rode `devzymer init` no projeto",
                root.display()
            )));
        }
        let store = Store::open(&db_path)?;
        let remote = build_remote_provider(&config.decisions)?;
        let router = DecisionRouter::new(
            config.decisions.clone(),
            remote,
            Some(std::sync::Arc::new(std::sync::Mutex::new(Store::open(
                &db_path,
            )?))),
        );
        let state = Arc::new(ServerState::default());
        let mut server = Server {
            root,
            store,
            config,
            router,
            watcher: None,
            state,
        };

        if server.config.index.watching {
            let scan = server.scan_options();
            if let Ok(watcher) = Watcher::start(&server.root, &db_path, scan) {
                server.watcher = Some(Arc::new(watcher));
            }
        }
        server.start_reconciliation();
        Ok(server)
    }

    pub fn with_store(root: impl Into<PathBuf>, config: Config, store: Store) -> Result<Server> {
        let root = root.into();
        let remote = build_remote_provider(&config.decisions)?;
        let router = DecisionRouter::new(config.decisions.clone(), remote, None);
        Ok(Server {
            root,
            store,
            config,
            router,
            watcher: None,
            state: Arc::new(ServerState::default()),
        })
    }

    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            exclude: self.config.index.exclude.clone(),
            exclude_secret_files: self.config.privacy.exclude_secret_files,
            max_file_size: self.config.index.max_file_size_bytes,
            follow_symlinks: self.config.index.follow_symlinks,
            respect_gitignore: self.config.index.respect_gitignore,
            include_hidden: self.config.index.include_hidden,
            languages: Vec::new(),
            max_workers: self.config.effective_workers(),
            max_duration_ms: Some(self.config.index.sync_budget_ms),
        }
    }

    /// Reconciles the index in the background, so a session that starts after
    /// files changed still converges without blocking the first request.
    fn start_reconciliation(&self) {
        let root = self.root.clone();
        let db_path = devzymer_core::paths::db_path(&root);
        let options = self.scan_options();
        let state = self.state.clone();
        state.reconciling.store(true, Ordering::Relaxed);
        let _ = std::thread::Builder::new()
            .name("devzymer-reconcile".to_string())
            .spawn(move || {
                let outcome = (|| -> Result<SyncReport> {
                    let store = Store::open(&db_path)?;
                    reconcile(&store, &root, &options)
                })();
                if let Ok(report) = outcome {
                    state.syncs.fetch_add(1, Ordering::Relaxed);
                    state
                        .last_sync_ms
                        .store(devzymer_core::util::now_millis(), Ordering::Relaxed);
                    state
                        .last_reconcile_ms
                        .store(devzymer_core::util::now_millis(), Ordering::Relaxed);
                    if let Ok(mut guard) = state.last_report.lock() {
                        *guard = Some(report);
                    }
                }
                state.reconciling.store(false, Ordering::Relaxed);
            });
    }

    /// Runs an incremental synchronization, bounded by the configured budget.
    pub fn sync_now(&self) -> Result<SyncReport> {
        let db_path = devzymer_core::paths::db_path(&self.root);
        let store = Store::open(&db_path)?;
        let options = self.scan_options();
        let report = reconcile(&store, &self.root, &options)?;
        self.state.syncs.fetch_add(1, Ordering::Relaxed);
        self.state
            .last_sync_ms
            .store(devzymer_core::util::now_millis(), Ordering::Relaxed);
        if let Ok(mut guard) = self.state.last_report.lock() {
            *guard = Some(report.clone());
        }
        Ok(report)
    }

    /// Reindexes the given files without walking the repository.
    pub fn reindex_files(&self, paths: &[PathBuf]) -> Result<SyncReport> {
        if paths.is_empty() {
            return Ok(SyncReport::default());
        }
        let db_path = devzymer_core::paths::db_path(&self.root);
        let store = Store::open(&db_path)?;
        let report = reindex_paths(&store, &self.root, paths)?;
        self.state.syncs.fetch_add(1, Ordering::Relaxed);
        self.state
            .last_sync_ms
            .store(devzymer_core::util::now_millis(), Ordering::Relaxed);
        Ok(report)
    }

    /// Returns the indexed files whose content changed on disk.
    pub fn stale_files(&self, paths: &[String]) -> Result<Vec<PathBuf>> {
        let mut stale = Vec::new();
        for rel in paths {
            let Some(record) = self.store.file_by_path(rel)? else {
                continue;
            };
            let absolute = self.root.join(rel);
            let Ok(metadata) = std::fs::metadata(&absolute) else {
                continue;
            };
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if mtime_ms != record.mtime_ms || metadata.len() != record.size_bytes {
                stale.push(absolute);
            }
        }
        Ok(stale)
    }

    /// Reindexes the files that changed, so a result is never based on a
    /// stale line reference.
    pub fn ensure_fresh(&self, paths: &[String]) -> Result<bool> {
        let stale = self.stale_files(paths)?;
        if stale.is_empty() {
            return Ok(false);
        }
        let report = self.reindex_files(&stale)?;
        if !report.errors.is_empty() {
            if let Ok(mut guard) = self.state.failures.lock() {
                *guard = report.errors.clone();
            }
        }
        Ok(true)
    }

    pub fn watcher_state(&self) -> Option<devzymer_indexer::watch::WatchSnapshot> {
        self.watcher.as_ref().map(|w| w.state().snapshot())
    }

    /// Handles one JSON-RPC message, returning the serialized response when
    /// the message expects one.
    pub async fn handle_line(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                return Some(
                    error_response(
                        Value::Null,
                        -32700,
                        &format!("parse error: {err}"),
                        Some("PARSE_ERROR"),
                    )
                    .to_string(),
                )
            }
        };
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        if method.is_empty() {
            return Some(
                error_response(
                    id.unwrap_or(Value::Null),
                    -32600,
                    "missing method",
                    Some("INVALID_REQUEST"),
                )
                .to_string(),
            );
        }

        if method.starts_with("notifications/") {
            return None;
        }

        let result = match method.as_str() {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": crate::tools::tool_definitions() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                let started = std::time::Instant::now();
                let output = match dispatch(self, &name, &arguments).await {
                    Ok(output) => output,
                    Err(err) => crate::tools::ToolOutput::error(err.code(), &err.to_string()),
                };
                self.record_call(&name, &output.text, output.is_error, started);
                Ok(output.to_json())
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            "shutdown" => Ok(json!({})),
            other => Err((
                format!("method not found: {other}"),
                -32601,
                "METHOD_NOT_FOUND",
            )),
        };

        match result {
            Ok(value) => {
                id.as_ref()?;
                Some(
                    json!({
                        "jsonrpc": "2.0",
                        "id": id.unwrap_or(Value::Null),
                        "result": value
                    })
                    .to_string(),
                )
            }
            Err((message, code, kind)) => Some(
                error_response(id.unwrap_or(Value::Null), code, &message, Some(kind)).to_string(),
            ),
        }
    }

    fn client_host(&self) -> Host {
        self.state
            .client
            .lock()
            .map(|host| *host)
            .unwrap_or(Host::Unknown)
    }

    fn record_call(&self, tool: &str, text: &str, is_error: bool, started: std::time::Instant) {
        let (mode, _) = self.config.routing_mode();
        if mode == devzymer_core::config::RoutingMode::Off {
            return;
        }
        let host = self.client_host();
        let outcome = signals::classify_text(tool, text, is_error);
        telemetry::append(
            &self.root,
            &self.config.routing,
            &Event {
                ts: devzymer_core::util::now_millis(),
                host: host.as_str().to_string(),
                host_source: if host == Host::Unknown {
                    "unknown".to_string()
                } else {
                    "client_info".to_string()
                },
                mode: mode.as_str().to_string(),
                kind: kind::MCP_CALL.to_string(),
                tool: tool.to_string(),
                reason: outcome.as_str().to_string(),
                latency_ms: Some(started.elapsed().as_millis() as u64),
                observed: observed::SERVER.to_string(),
                ..Event::default()
            },
        );
    }

    fn initialize(&self, params: &Value) -> Value {
        if let Some(name) = params
            .get("clientInfo")
            .and_then(|info| info.get("name"))
            .and_then(|name| name.as_str())
        {
            if let Ok(mut host) = self.state.client.lock() {
                *host = Host::from_client_name(name);
            }
        }
        let requested = params
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let protocol_version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
            requested.to_string()
        } else {
            self.config.mcp.protocol_version.clone()
        };
        json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": { "listChanged": false }
            },
            "serverInfo": {
                "name": self.config.mcp.server_name.clone(),
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Local code index. Start repository discovery with context_resolve, read only the returned range with context_read, and check dependents with context_impact before changing shared code. Native search remains the right tool for logs, command output, unindexed or newer-than-index content, and when a lookup reports nao_encontrado."
        })
    }
}

fn error_response(id: Value, code: i64, message: &str, kind: Option<&str>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(kind) = kind {
        error["data"] = json!({ "code": kind });
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// Runs the MCP server over stdio until the client closes the stream.
fn start_heartbeat(
    root: PathBuf,
    config: &Config,
    state: Arc<ServerState>,
) -> Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)> {
    if config.routing_mode().0 == devzymer_core::config::RoutingMode::Off {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let started_ms = devzymer_core::util::now_millis();
    let handle = std::thread::Builder::new()
        .name("devzymer-heartbeat".to_string())
        .spawn(move || {
            let mut elapsed = HEARTBEAT_INTERVAL_MS;
            while !flag.load(Ordering::Relaxed) {
                if elapsed >= HEARTBEAT_INTERVAL_MS {
                    elapsed = 0;
                    let host = state
                        .client
                        .lock()
                        .map(|host| *host)
                        .unwrap_or(Host::Unknown);
                    health::write_heartbeat(
                        &root,
                        &Heartbeat {
                            pid: std::process::id(),
                            started_ms,
                            updated_ms: devzymer_core::util::now_millis(),
                            host: host.as_str().to_string(),
                        },
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
                elapsed += 250;
            }
        })
        .ok()?;
    Some((stop, handle))
}

const HEARTBEAT_INTERVAL_MS: u64 = 30_000;

pub async fn serve_stdio(server: Server) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let heartbeat = start_heartbeat(server.root.clone(), &server.config, server.state.clone());

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            break;
        }
        if let Some(response) = server.handle_line(&line).await {
            stdout.write_all(response.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    if let Some((stop, handle)) = heartbeat {
        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        health::clear_heartbeat(&server.root, std::process::id());
    }
    Ok(())
}

/// Locates the project root the server should operate on.
///
/// The explicit argument wins and is never replaced by an ancestor. When it
/// holds no index, the project above it is used, because that is what a host
/// means by the current project. Without an explicit argument, the documented
/// host context and the working directory are tried, and only a directory that
/// holds an index is accepted. The user home is never a workspace.
pub fn resolve_root(explicit: Option<&Path>) -> Result<PathBuf> {
    let policy = devzymer_core::workspace_root::Policy {
        allow_broad: broad_root_allowed(),
        allow_unversioned: false,
    };

    if let Some(path) = explicit {
        let candidate = devzymer_core::workspace_root::canonical(path)?;
        if devzymer_core::workspace_root::is_broad(&candidate) && !policy.allow_broad {
            return Err(devzymer_core::Error::WorkspaceTooBroad(format!(
                "{} e o diretorio do usuario ou um ancestral dele; o servidor nao \
                 opera sobre esse caminho",
                candidate.display()
            )));
        }
        if has_index(&candidate) {
            return Ok(candidate);
        }
        // The host asked for the current project. When the directory itself has
        // no index, the project above it is the one the host meant.
        if let Ok(discovered) = devzymer_core::workspace_root::discover(&candidate, policy) {
            if has_index(&discovered.root) {
                return Ok(discovered.root);
            }
        }
        return Err(devzymer_core::Error::WorkspaceNotResolved(format!(
            "nenhum indice devzymer em {}; rode `devzymer init --root {}`",
            candidate.display(),
            candidate.display()
        )));
    }

    let mut seeds: Vec<PathBuf> = Vec::new();
    if let Some(dir) = devzymer_core::paths::host_project_dir() {
        seeds.push(dir);
    }
    if let Some(dir) = devzymer_core::paths::claude_project_dir() {
        seeds.push(dir);
    }
    seeds.push(devzymer_core::paths::current_dir());

    for seed in seeds {
        if let Ok(discovered) = devzymer_core::workspace_root::discover(&seed, policy) {
            if has_index(&discovered.root) {
                return Ok(discovered.root);
            }
        }
    }

    Err(devzymer_core::Error::WorkspaceNotResolved(
        "nenhum workspace com indice devzymer foi encontrado; o host precisa \
         fornecer o diretorio do projeto"
            .to_string(),
    ))
}

/// True when the operator allows a workspace at the user home or above it.
fn broad_root_allowed() -> bool {
    matches!(
        std::env::var("DEVZYMER_ALLOW_BROAD_ROOT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// True when the directory holds a devzymer index.
fn has_index(root: &Path) -> bool {
    devzymer_core::paths::project_dir(root)
        .join("index.sqlite3")
        .exists()
}
