use std::path::Path;

use devzymer_core::model::Symbol;
use devzymer_core::util::estimate_tokens;
use devzymer_core::Result;
use devzymer_search::read::{read_excerpt, ExcerptRequest};
use devzymer_search::resolve::ResolveQuery;
use devzymer_search::Resolver;
use serde_json::{json, Value};

use crate::patch::{apply_patch, PatchRequest};
use crate::server::Server;

const AUTO_SYNC_COOLDOWN_MS: u64 = 5_000;

pub struct ToolOutput {
    pub text: String,
    pub structured: Value,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(text: impl Into<String>, structured: Value) -> Self {
        Self {
            text: text.into(),
            structured,
            is_error: false,
        }
    }

    pub fn error(code: &str, message: &str) -> Self {
        Self {
            text: format!("ERR {code}: {message}"),
            structured: json!({ "error": { "code": code, "message": message } }),
            is_error: true,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "content": [{ "type": "text", "text": self.text }],
            "structuredContent": self.structured,
            "isError": self.is_error,
        })
    }
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "context_resolve",
            "description": "Locate where a symbol, file or behavior lives in the indexed repository. Accepts a symbol name, qualified name, file name, path or short natural-language description, and returns ranked candidates (path, symbol, line range) without scanning the repository. Partial matches rank lower, and an `ambiguidade:` reply asks for a qualified name. A ranked hit is a lead, not proof: confirm it with context_read. `nao_encontrado:` means not indexed or not matched, not proven absent; use native search then, and for logs, command output or content newer than the index.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Symbol name, file name, path or natural language description." },
                    "path": { "type": "string", "description": "Restrict the search to a repository relative file path or directory. Only symbols inside it are returned. A path that matches no file returns nothing, and a path outside the workspace is rejected." },
                    "language": { "type": "string", "description": "Filter by language, for example typescript or rust." },
                    "diagnostics": { "type": "boolean", "description": "Include the ranking evidence of each candidate. Default false." },
                    "limit": { "type": "integer", "description": "Maximum number of results. Default 20." }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "context_read",
            "description": "Read a line range or a named symbol from an indexed file, numbered, with the sha that context_patch requires. Use it after context_resolve instead of reading the whole file. A `| truncado |` header means the excerpt stopped at max_bytes: continue from the stated line. Read a whole file natively only when you need to edit it with a native editor.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Repository relative file path." },
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" },
                    "symbol": { "type": "string", "description": "Symbol name or qualified name to locate the range." },
                    "max_bytes": { "type": "integer", "description": "Maximum bytes to return. Default 24000." },
                    "numbered": { "type": "boolean", "description": "Prefix each line with its line number. Default true." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "context_impact",
            "description": "List callers, dependents and the blast radius of a symbol or file, before changing shared code. Each edge is labeled confirmed (static rule), heuristic (name match) or unresolved (target unknown). The label rates the link, not the correctness of the code, and dynamic dispatch can leave the graph partial: verify important edges with context_read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name or qualified name." },
                    "path": { "type": "string", "description": "Repository relative file path." },
                    "depth": { "type": "integer", "description": "Traversal depth. Default 2." }
                }
            }
        }),
        json!({
            "name": "context_patch",
            "description": "Apply a small change to an existing file without rewriting it. Requires the expected_sha returned by context_read. The edit is rejected when the file changed, when the range is outside the excerpt that was read, or when it would replace the whole file. Writes atomically.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Repository relative file path." },
                    "symbol": { "type": "string", "description": "Symbol whose body should be replaced." },
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" },
                    "replacement": { "type": "string", "description": "Replacement text for the selected range." },
                    "expected_sha": { "type": "string", "description": "File hash reported by context_read. The patch is rejected when it no longer matches, and it is mandatory." },
                    "dry_run": { "type": "boolean", "description": "Validate the edit without writing it." },
                    "allow_whole_file": { "type": "boolean", "description": "Authorize replacing the entire file. Default false, so an edit stays minimal." }
                },
                "required": ["path", "replacement", "expected_sha"]
            }
        }),
        json!({
            "name": "context_status",
            "description": "Report index freshness, coverage, watcher state, pending changes, skipped files and indexing failures. Call it when a result looks stale or before trusting the index in a new workspace; it is not needed before every lookup.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "context_sync",
            "description": "Synchronize the local index with the files on disk. Call it after creating, renaming or deleting files, or when a lookup returns a stale location.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "full": { "type": "boolean", "description": "Force a full rebuild of the index. Default false." },
                    "budget_ms": { "type": "integer", "description": "Time budget for the scan, in milliseconds. Default 2000." }
                }
            }
        }),
        json!({
            "name": "context_decide",
            "description": "Request a bounded routing decision when deterministic rules are insufficient. Falls back to local rules when the external provider is unavailable.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "One of: complexity, retrieval_strategy, need_more_context, evidence_sufficient, next_source, escalate, semantic_fallback, workflow."
                    },
                    "state": { "type": "string", "description": "Short, non sensitive summary of the current evidence." },
                    "options": { "type": "array", "items": { "type": "string" } },
                    "hint": { "type": "string" }
                },
                "required": ["task", "state"]
            }
        }),
    ]
}

/// The ceiling of a single response, whatever the caller asks for.
const HARD_MAX_FRAGMENT_BYTES: u64 = 256 * 1024;

pub async fn dispatch(server: &Server, name: &str, args: &Value) -> Result<ToolOutput> {
    match name {
        "context_resolve" => resolve(server, args),
        "context_read" => read(server, args),
        "context_impact" => impact(server, args),
        "context_patch" => patch(server, args),
        "context_status" => status(server),
        "context_sync" => sync(server, args),
        "context_decide" => decide(server, args).await,
        other => Ok(ToolOutput::error(
            "UNKNOWN_TOOL",
            &format!("unknown tool: {other}"),
        )),
    }
}

fn resolve(server: &Server, args: &Value) -> Result<ToolOutput> {
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .unwrap_or_default()
        .to_string();
    if query.trim().is_empty() {
        return Ok(ToolOutput::error("INVALID_INPUT", "query is required"));
    }
    let limit = args
        .get("limit")
        .and_then(|l| l.as_u64())
        .unwrap_or(server.config.search.max_results as u64) as usize;
    let diagnostics = args
        .get("diagnostics")
        .and_then(|d| d.as_bool())
        .unwrap_or(false);
    let language = args
        .get("language")
        .and_then(|l| l.as_str())
        .and_then(devzymer_core::model::Language::from_display);
    let path_filter = args
        .get("path")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    if let Some(path) = path_filter.as_ref() {
        if !Resolver::path_is_inside_workspace(path) {
            return Ok(ToolOutput::error(
                "PATH_OUTSIDE_WORKSPACE",
                &format!("{path} esta fora do workspace"),
            ));
        }
    }

    let resolver = Resolver::new(&server.store, &server.config);
    let build_query = || ResolveQuery {
        query: query.clone(),
        path: path_filter.clone(),
        language,
        limit,
        candidate_limit: 0,
        diagnostics,
    };

    let mut set = resolver.resolve(&build_query())?;
    let mut refreshed = false;

    // Never report a line reference that the file no longer supports.
    if !set.results.is_empty() {
        let paths: Vec<String> = {
            let mut seen: Vec<String> = Vec::new();
            for r in &set.results {
                if !seen.contains(&r.symbol.path) {
                    seen.push(r.symbol.path.clone());
                }
            }
            seen
        };
        if server.ensure_fresh(&paths)? {
            refreshed = true;
            set = resolver.resolve(&build_query())?;
        }
    }

    if set.results.is_empty() && set.files.is_empty() {
        // The lookup failed: the index may be behind the filesystem.
        let report = auto_sync(server)?;
        if report.changed() {
            refreshed = true;
            set = resolver.resolve(&build_query())?;
        }
    }

    if set.results.is_empty() && set.files.is_empty() {
        // A well-formed search with zero matches is a successful outcome,
        // not a tool error: the payload says so explicitly.
        let mut message = format!(
            "nao_encontrado: '{query}' nao corresponde a nenhum simbolo ou arquivo indexado"
        );
        if refreshed {
            message.push_str(" (o indice foi sincronizado antes de responder)");
        }
        return Ok(ToolOutput::ok(
            message,
            json!({
                "query": query,
                "count": 0,
                "intent": intent_of(&query),
                "not_found": true,
                "refreshed": refreshed,
                "cache": set.cache.display(),
                "files": [],
                "results": []
            }),
        ));
    }

    let mut lines: Vec<String> = Vec::new();
    for file in &set.files {
        lines.push(format!(
            "arquivo: {} | {} símbolo(s) | {}",
            file.path, file.symbols, file.reason
        ));
    }
    let mut items: Vec<Value> = Vec::new();
    for r in &set.results {
        lines.push(format!(
            "{}:{}-{} | {} | {} | {}",
            r.symbol.path,
            r.symbol.span.start_line,
            r.symbol.span.end_line,
            r.symbol.qualified_name,
            r.symbol.kind,
            r.confidence.display()
        ));
        let mut item = json!({
            "path": r.symbol.path,
            "name": r.symbol.name,
            "qualified_name": r.symbol.qualified_name,
            "kind": r.symbol.kind.display(),
            "start_line": r.symbol.span.start_line,
            "end_line": r.symbol.span.end_line,
            "signature": bounded_signature(r.symbol.signature.as_deref()),
            "confidence": r.confidence.display(),
            "source": r.source.display(),
            "score": r.score,
            "matched_terms": r.matches,
            "identity_matches": r.identity_matches
        });
        if diagnostics {
            item["evidence"] = json!((r.evidence * 1000.0).round() / 1000.0);
            item["sequence_cover"] = json!(r.sequence_cover);
        }
        items.push(item);
    }

    if let Some(ambiguity) = &set.ambiguity {
        lines.insert(
            0,
            format!(
                "ambiguidade: '{}' tem {} definições; use o nome qualificado para desambiguar",
                ambiguity.name, ambiguity.count
            ),
        );
    }

    Ok(ToolOutput::ok(
        lines.join("\n"),
        json!({
            "query": query,
            "count": set.results.len(),
            "intent": intent_of(&query),
            "refreshed": refreshed,
            "cache": set.cache.display(),
            "files": set.files,
            "ambiguity": set.ambiguity,
            "results": items,
            "evidence": set.evidence
        }),
    ))
}

/// The signature of a symbol can hold the whole body of a large function.
/// The answer carries a bounded excerpt of it; the complete source is read
/// with context_read.
fn bounded_signature(signature: Option<&str>) -> Option<String> {
    const LIMIT: usize = 240;
    let text = signature?;
    if text.chars().count() <= LIMIT {
        return Some(text.to_string());
    }
    let excerpt: String = text.chars().take(LIMIT).collect();
    Some(format!("{excerpt}..."))
}

/// Deterministic classification of how the caller phrased the query. It is
/// diagnostic telemetry for retrieval reviews and never changes ranking.
fn intent_of(query: &str) -> &'static str {
    let trimmed = query.trim();
    if trimmed.contains('/') || trimmed.contains('\\') {
        return "path";
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() == 1 {
        if trimmed
            .rsplit_once('.')
            .is_some_and(|(_, ext)| !ext.is_empty() && !ext.contains(' '))
        {
            return "path";
        }
        if trimmed.as_bytes()[1..]
            .iter()
            .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
        {
            return "identifier";
        }
    }
    "behavior"
}

/// Synchronizes the index when a lookup fails, respecting a cooldown so a
/// repeated miss does not rescan the repository.
fn auto_sync(server: &Server) -> Result<devzymer_indexer::sync::SyncReport> {
    let now = devzymer_core::util::now_millis();
    let last = server
        .state
        .last_sync_ms
        .load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(last) < AUTO_SYNC_COOLDOWN_MS {
        return Ok(devzymer_indexer::sync::SyncReport::default());
    }
    server.sync_now()
}

fn read(server: &Server, args: &Value) -> Result<ToolOutput> {
    let path = args
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();
    if path.trim().is_empty() {
        return Ok(ToolOutput::error("INVALID_INPUT", "path is required"));
    }
    // The caller cannot raise the budget above the configured limit, and the
    // server keeps a hard ceiling, so one request never returns megabytes.
    let configured = server.config.search.max_fragment_bytes as u64;
    let max_bytes = args
        .get("max_bytes")
        .and_then(|m| m.as_u64())
        .map(|requested| requested.min(configured).min(HARD_MAX_FRAGMENT_BYTES))
        .unwrap_or_else(|| configured.min(HARD_MAX_FRAGMENT_BYTES)) as usize;

    let (start_line, end_line) = match args.get("symbol").and_then(|s| s.as_str()) {
        Some(reference) => match find_symbol(server, reference)? {
            Some(symbol) => (symbol.span.start_line, symbol.span.end_line),
            None => {
                return Ok(ToolOutput::error(
                    "SYMBOL_NOT_FOUND",
                    &format!("symbol '{reference}' is not in the index"),
                ))
            }
        },
        None => {
            let start = args.get("start_line").and_then(|s| s.as_u64());
            let end = args.get("end_line").and_then(|s| s.as_u64());
            match (start, end) {
                (Some(start), Some(end)) => (start.max(1) as u32, end.max(1).max(start) as u32),
                (Some(start), None) => (start.max(1) as u32, start.max(1) as u32),
                (None, Some(end)) => (1, end.max(1) as u32),
                (None, None) => (1, u32::MAX),
            }
        }
    };

    let excerpt = read_excerpt(
        &server.root,
        &ExcerptRequest {
            path: path.clone(),
            start_line,
            end_line,
            max_bytes,
            max_lines: server.config.search.snippet_max_lines,
        },
    )?;

    let excerpt_key = excerpt.path.clone();
    let record = crate::server::ReadRecord {
        start_line: excerpt.start_line,
        end_line: excerpt.end_line,
        sha: excerpt.sha.clone(),
        truncated: excerpt.truncated,
    };
    if let Ok(mut reads) = server.state.reads.lock() {
        reads.insert(excerpt_key, record);
    }

    let numbered = args
        .get("numbered")
        .and_then(|n| n.as_bool())
        .unwrap_or(true);
    let body = if numbered {
        excerpt
            .text
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{:>6} | {}", excerpt.start_line as usize + i, line))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        excerpt.text.clone()
    };

    // The header states the range that was really delivered, so a caller never
    // addresses a block it did not receive.
    let truncation = if excerpt.truncated {
        format!(
            " | truncado | continuacao a partir da linha {}",
            excerpt.next_start_line.unwrap_or(excerpt.end_line + 1)
        )
    } else {
        String::new()
    };
    let text = format!(
        "{}:{}-{} sha={} bytes={} tokens~{}{}\n{}",
        excerpt.path,
        excerpt.start_line,
        excerpt.end_line,
        excerpt.sha,
        excerpt.bytes,
        estimate_tokens(&body),
        truncation,
        body
    );

    Ok(ToolOutput::ok(
        text,
        json!({
            "path": excerpt.path,
            "start_line": excerpt.start_line,
            "end_line": excerpt.end_line,
            "sha": excerpt.sha,
            "bytes": excerpt.bytes,
            "truncated": excerpt.truncated,
            "next_start_line": excerpt.next_start_line,
            "text": body
        }),
    ))
}

fn impact(server: &Server, args: &Value) -> Result<ToolOutput> {
    let depth = args
        .get("depth")
        .and_then(|d| d.as_u64())
        .unwrap_or(2)
        .clamp(1, 5) as usize;

    let graph = devzymer_graph::GraphQuery::new(&server.store);

    if let Some(reference) = args.get("symbol").and_then(|s| s.as_str()) {
        let Some(symbol) = find_symbol(server, reference)? else {
            // The query may name a file instead of a symbol.
            let resolver = Resolver::new(&server.store, &server.config);
            let set = resolver.resolve(&ResolveQuery {
                query: reference.to_string(),
                path: None,
                language: None,
                limit: 5,
                candidate_limit: 0,
                diagnostics: false,
            })?;
            if let Some(file) = set.files.first() {
                let dependents = graph.dependents(&file.path)?;
                let dependencies = graph.dependencies(&file.path)?;
                let defines = graph.defines(&file.path)?;
                let mut lines = vec![format!(
                    "impact of file {} | {} símbolo(s) definido(s) | {} dependente(s) | {} dependência(s)",
                    file.path,
                    defines.len(),
                    dependents.len(),
                    dependencies.len()
                )];
                let mut dependent_items: Vec<Value> = Vec::new();
                for relation in &dependents {
                    if let devzymer_core::model::Entity::File(f) = &relation.src {
                        let symbols = server
                            .store
                            .file_by_path(f)
                            .ok()
                            .flatten()
                            .map(|record| record.symbol_count)
                            .unwrap_or(0);
                        lines.push(format!("  dependente: {f} | {symbols} símbolo(s)"));
                        dependent_items.push(json!({
                            "path": f,
                            "symbols": symbols,
                            "confidence": relation.confidence.display()
                        }));
                    }
                }
                let mut dependency_items: Vec<Value> = Vec::new();
                for relation in &dependencies {
                    if let devzymer_core::model::Entity::File(f) = &relation.dst {
                        let symbols = server
                            .store
                            .file_by_path(f)
                            .ok()
                            .flatten()
                            .map(|record| record.symbol_count)
                            .unwrap_or(0);
                        lines.push(format!("  depende de: {f} | {symbols} símbolo(s)"));
                        dependency_items.push(json!({
                            "path": f,
                            "symbols": symbols,
                            "confidence": relation.confidence.display()
                        }));
                    }
                }
                if dependents.is_empty() && dependencies.is_empty() {
                    lines.push(
                        "nenhuma relação de dependência foi estabelecida para este arquivo"
                            .to_string(),
                    );
                }
                lines.push(format!(
                    "cobertura: dependentes {} | dependências {} | a dependência entre arquivos é resolvida por import, e chamadas diretas entre arquivos não são contadas aqui",
                    dependents.len(),
                    dependencies.len()
                ));
                return Ok(ToolOutput::ok(
                    lines.join("\n"),
                    json!({
                        "file": file.path,
                        "defines": defines.len(),
                        "dependents": dependents.len(),
                        "dependencies": dependencies.len(),
                        "dependent_files": dependent_items,
                        "dependency_files": dependency_items,
                        "matched_by": file.reason
                    }),
                ));
            }
            if let Some(candidate) = set.results.first() {
                let report = graph.impact(&candidate.symbol.id, depth)?;
                let mut lines = vec![format!(
                    "impact of {} ({}:{}) | {} relação(ões)",
                    candidate.symbol.qualified_name,
                    candidate.symbol.path,
                    candidate.symbol.span.start_line,
                    report.relations.len()
                )];
                for related in &report.symbols {
                    lines.push(format!(
                        "{}:{}-{} | {} | {}",
                        related.path,
                        related.span.start_line,
                        related.span.end_line,
                        related.qualified_name,
                        related.kind
                    ));
                }
                if report.symbols.is_empty() {
                    lines.push(
                        "nenhuma relação de chamada ou dependência foi estabelecida para este símbolo"
                            .to_string(),
                    );
                }
                let items: Vec<Value> = report
                    .symbols
                    .iter()
                    .map(|s| {
                        json!({
                            "path": s.path,
                            "qualified_name": s.qualified_name,
                            "kind": s.kind.display(),
                            "start_line": s.span.start_line,
                            "end_line": s.span.end_line
                        })
                    })
                    .collect();
                return Ok(ToolOutput::ok(
                    lines.join("\n"),
                    json!({
                        "target": candidate.symbol.qualified_name,
                        "depth": depth,
                        "related": items,
                        "relations": report.relations.len(),
                        "matched_by": candidate.source.display()
                    }),
                ));
            }
            return Ok(ToolOutput::error(
                "SYMBOL_NOT_FOUND",
                &format!("symbol or file '{reference}' is not in the index"),
            ));
        };
        let report = graph.impact(&symbol.id, depth)?;
        let mut lines = vec![format!(
            "impact of {} ({}:{}): {} símbolo(s) relacionado(s), {} relação(ões)",
            symbol.qualified_name,
            symbol.path,
            symbol.span.start_line,
            report.symbols.len(),
            report.relations.len()
        )];
        let mut relation_items: Vec<Value> = Vec::new();
        for relation in &report.relations {
            let source = match &relation.src {
                devzymer_core::model::Entity::Symbol(id) => server
                    .store
                    .symbol_by_id(id.as_str())
                    .ok()
                    .flatten()
                    .map(|s| s.qualified_name)
                    .unwrap_or_else(|| id.0.clone()),
                devzymer_core::model::Entity::File(path) => path.clone(),
                devzymer_core::model::Entity::External(name) => name.clone(),
            };
            let target = match &relation.dst {
                devzymer_core::model::Entity::Symbol(id) => server
                    .store
                    .symbol_by_id(id.as_str())
                    .ok()
                    .flatten()
                    .map(|s| s.qualified_name)
                    .unwrap_or_else(|| id.0.clone()),
                devzymer_core::model::Entity::File(path) => path.clone(),
                devzymer_core::model::Entity::External(name) => name.clone(),
            };
            lines.push(format!(
                "  {} | {} -> {} | {}",
                relation.kind.display(),
                source,
                target,
                relation.confidence.display()
            ));
            relation_items.push(json!({
                "kind": relation.kind.display(),
                "source": source,
                "target": target,
                "confidence": relation.confidence.display(),
                "evidence": relation.evidence
            }));
        }
        for related in &report.symbols {
            lines.push(format!(
                "{}:{}-{} | {} | {}",
                related.path,
                related.span.start_line,
                related.span.end_line,
                related.qualified_name,
                related.kind
            ));
        }
        if report.symbols.is_empty() {
            lines.push(
                "nenhuma relação de chamada ou dependência foi estabelecida para este símbolo"
                    .to_string(),
            );
        }
        let confirmed = report
            .relations
            .iter()
            .filter(|r| r.confidence == devzymer_core::model::Confidence::Confirmed)
            .count();
        lines.push(format!(
            "cobertura: {} relação(ões), {} confirmada(s) e {} heurística(s); chamadas são resolvidas por nome e podem incluir chamadas de mesmo nome em outro escopo",
            report.relations.len(),
            confirmed,
            report.relations.len().saturating_sub(confirmed)
        ));
        let items: Vec<Value> = report
            .symbols
            .iter()
            .map(|s| {
                json!({
                    "path": s.path,
                    "qualified_name": s.qualified_name,
                    "kind": s.kind.display(),
                    "start_line": s.span.start_line,
                    "end_line": s.span.end_line
                })
            })
            .collect();
        return Ok(ToolOutput::ok(
            lines.join("\n"),
            json!({
                "target": symbol.qualified_name,
                "depth": depth,
                "related": items,
                "relations": report.relations.len(),
                "relation_details": relation_items,
                "confirmed_relations": confirmed,
                "truncated": report.truncated
            }),
        ));
    }

    if let Some(path) = args.get("path").and_then(|p| p.as_str()) {
        let dependents = graph.dependents(path)?;
        let dependencies = graph.dependencies(path)?;
        let mut lines = vec![format!("dependentes de {path}")];
        for r in &dependents {
            if let devzymer_core::model::Entity::File(f) = &r.src {
                lines.push(format!("  {f}"));
            }
        }
        lines.push(format!("dependências de {path}"));
        for r in &dependencies {
            if let devzymer_core::model::Entity::File(f) = &r.dst {
                lines.push(format!("  {f}"));
            }
        }
        return Ok(ToolOutput::ok(
            lines.join("\n"),
            json!({
                "path": path,
                "dependents": dependents.len(),
                "dependencies": dependencies.len()
            }),
        ));
    }

    Ok(ToolOutput::error(
        "INVALID_INPUT",
        "either symbol or path is required",
    ))
}

fn patch(server: &Server, args: &Value) -> Result<ToolOutput> {
    let path = args
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string();
    if path.trim().is_empty() {
        return Ok(ToolOutput::error("INVALID_INPUT", "path is required"));
    }
    // A path outside the workspace can never be patched, so it is reported
    // before any other argument check: the caller gets the specific reason
    // instead of a revision complaint.
    if devzymer_core::util::resolve_inside(&server.root, std::path::Path::new(&path)).is_err() {
        return Ok(ToolOutput::error(
            "PATCH_OUTSIDE_WORKSPACE",
            &format!("{path} esta fora do workspace"),
        ));
    }
    let replacement = args
        .get("replacement")
        .and_then(|r| r.as_str())
        .unwrap_or_default()
        .to_string();

    let (start_line, end_line) = match args.get("symbol").and_then(|s| s.as_str()) {
        Some(reference) => match find_symbol(server, reference)? {
            Some(symbol) => (symbol.span.start_line, symbol.span.end_line),
            None => {
                return Ok(ToolOutput::error(
                    "SYMBOL_NOT_FOUND",
                    &format!("symbol '{reference}' is not in the index"),
                ))
            }
        },
        None => {
            let start = args.get("start_line").and_then(|s| s.as_u64()).unwrap_or(0) as u32;
            let end = args
                .get("end_line")
                .and_then(|s| s.as_u64())
                .unwrap_or(start as u64) as u32;
            if start == 0 {
                return Ok(ToolOutput::error(
                    "INVALID_INPUT",
                    "start_line or symbol is required",
                ));
            }
            (start, end)
        }
    };

    let expected_sha = args
        .get("expected_sha")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if expected_sha.is_empty() {
        return Ok(ToolOutput::error(
            "PATCH_CONTENT_MISMATCH",
            "expected_sha is required: use the hash returned by context_read",
        ));
    }

    let request = PatchRequest {
        path: path.clone(),
        start_line,
        end_line,
        replacement,
        expected_sha,
        dry_run: args
            .get("dry_run")
            .and_then(|d| d.as_bool())
            .unwrap_or(false),
        allow_whole_file: args
            .get("allow_whole_file")
            .and_then(|d| d.as_bool())
            .unwrap_or(false),
    };

    let bound = last_read(server, &path);
    match apply_patch(&server.root, &request, bound.as_ref()) {
        Ok(outcome) => {
            let text = format!(
                "OK | {}:{}->{} | sha {} -> {} | {} linhas -> {} linhas{}",
                outcome.path,
                outcome.replaced_lines,
                outcome.lines_after,
                outcome.previous_sha,
                outcome.new_sha,
                outcome.lines_before,
                outcome.lines_after,
                if outcome.dry_run { " | dry-run" } else { "" }
            );
            Ok(ToolOutput::ok(
                text,
                json!({
                    "path": outcome.path,
                    "previous_sha": outcome.previous_sha,
                    "new_sha": outcome.new_sha,
                    "lines_before": outcome.lines_before,
                    "lines_after": outcome.lines_after,
                    "replaced_lines": outcome.replaced_lines,
                    "dry_run": outcome.dry_run,
                    "diff": outcome.diff_preview
                }),
            ))
        }
        Err(err) => Ok(ToolOutput::error(err.code(), &err.to_string())),
    }
}

fn status(server: &Server) -> Result<ToolOutput> {
    let stats = server.store.stats()?;
    let integrity = server.store.integrity_check()?;
    let watcher = server.watcher_state();
    let last_report = server
        .state
        .last_report
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    let failures = server
        .state
        .failures
        .lock()
        .ok()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let pending = watcher.as_ref().map(|w| w.pending).unwrap_or(0);

    let languages = stats
        .languages
        .iter()
        .map(|(l, c)| format!("{l}={c}"))
        .collect::<Vec<_>>()
        .join(" ");

    let text = format!(
        "root: {}\nindex: {} files, {} symbols, {} relations, {} bytes | revision {} | schema {} | parser {}\nintegrity: {} | languages: {}\nfreshness: sync {} | reconciling {} | pending {} | skipped {} | falhas {}",
        server.root.display(),
        stats.files,
        stats.symbols,
        stats.relations,
        stats.indexed_bytes,
        stats.revision,
        stats.schema_version,
        stats.parser_version,
        integrity,
        languages,
        server.state.last_sync_ms.load(std::sync::atomic::Ordering::Relaxed),
        server.state.reconciling.load(std::sync::atomic::Ordering::Relaxed),
        pending,
        last_report.as_ref().map(|r| r.skipped.len()).unwrap_or(0),
        failures.len()
    );

    let index_path = devzymer_core::paths::db_path(&server.root);
    let mut warnings: Vec<String> = Vec::new();
    if !index_path.exists() {
        warnings.push(format!(
            "não há índice em {}; execute `devzymer init` nessa pasta",
            index_path.display()
        ));
    }

    Ok(ToolOutput::ok(
        text,
        json!({
            "root": server.root.display().to_string(),
            "index_path": index_path.display().to_string(),
            "files": stats.files,
            "symbols": stats.symbols,
            "relations": stats.relations,
            "indexed_bytes": stats.indexed_bytes,
            "revision": stats.revision,
            "schema_version": stats.schema_version,
            "parser_version": stats.parser_version,
            "last_index_ms": stats.last_index_ms,
            "integrity": integrity,
            "languages": stats.languages,
            "freshness": {
                "last_sync_ms": server.state.last_sync_ms.load(std::sync::atomic::Ordering::Relaxed),
                "last_reconcile_ms": server.state.last_reconcile_ms.load(std::sync::atomic::Ordering::Relaxed),
                "reconciling": server.state.reconciling.load(std::sync::atomic::Ordering::Relaxed),
                "syncs": server.state.syncs.load(std::sync::atomic::Ordering::Relaxed),
                "pending_changes": pending,
                "skipped": last_report.as_ref().map(|r| r.skipped.clone()).unwrap_or_default(),
                "failures": failures,
                "truncated": last_report.as_ref().map(|r| r.truncated).unwrap_or(false)
            },
            "watcher": watcher,
            "warnings": warnings,
            "decisions_enabled": server.config.decisions.enabled,
            "fts": server.store.fts_available()
        }),
    ))
}

fn sync(server: &Server, args: &Value) -> Result<ToolOutput> {
    let full = args.get("full").and_then(|f| f.as_bool()).unwrap_or(false);
    let report = if full {
        let db_path = devzymer_core::paths::db_path(&server.root);
        let store = devzymer_core::store::Store::open(&db_path)?;
        let options = devzymer_indexer::scan::ScanOptions {
            max_duration_ms: args.get("budget_ms").and_then(|b| b.as_u64()),
            ..server.scan_options()
        };
        let indexer = devzymer_indexer::index::Indexer::new(&store, server.root.clone());
        let index_options = devzymer_indexer::index::IndexOptions {
            root: server.root.clone(),
            scan: options,
            force_full: true,
            chunk_size: 256,
        };
        let index_report = indexer.index(&index_options)?;
        let graph_report = devzymer_graph::build_graph(&store)?;
        server
            .state
            .syncs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        server.state.last_sync_ms.store(
            devzymer_core::util::now_millis(),
            std::sync::atomic::Ordering::Relaxed,
        );
        devzymer_indexer::sync::SyncReport {
            added: vec![format!(
                "{} arquivos reindexados",
                index_report.files_indexed
            )],
            modified: Vec::new(),
            removed: Vec::new(),
            unchanged: index_report.files_unchanged,
            symbols: index_report.symbols,
            errors: index_report.errors,
            skipped: index_report
                .skipped
                .iter()
                .map(|s| format!("{} ({:?})", s.rel_path, s.reason))
                .collect(),
            duration_ms: index_report.duration_ms,
            truncated: false,
        }
        .tap_graph(graph_report.relations)
    } else {
        server.sync_now()?
    };

    let text = format!(
        "OK | sincronizado | +{} ~{} -{} | {} símbolos | {} ms{}",
        report.added.len(),
        report.modified.len(),
        report.removed.len(),
        report.symbols,
        report.duration_ms,
        if report.truncated { " | truncado" } else { "" }
    );

    Ok(ToolOutput::ok(
        text,
        json!({
            "added": report.added,
            "modified": report.modified,
            "removed": report.removed,
            "unchanged": report.unchanged,
            "symbols": report.symbols,
            "errors": report.errors,
            "skipped": report.skipped,
            "duration_ms": report.duration_ms,
            "truncated": report.truncated
        }),
    ))
}

trait TapGraph {
    fn tap_graph(self, relations: usize) -> Self;
}

impl TapGraph for devzymer_indexer::sync::SyncReport {
    fn tap_graph(self, _relations: usize) -> Self {
        self
    }
}

async fn decide(server: &Server, args: &Value) -> Result<ToolOutput> {
    use devzymer_decisions::schema::{DecisionRequest, DecisionTask};

    let task_name = args
        .get("task")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    let Some(task) = DecisionTask::from_name(task_name) else {
        return Ok(ToolOutput::error(
            "INVALID_INPUT",
            &format!("unknown decision task: {task_name}"),
        ));
    };
    let state = args
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .to_string();
    if state.len() > server.config.decisions.max_state_chars {
        return Ok(ToolOutput::error(
            "RESOURCE_LIMIT_EXCEEDED",
            "state exceeds the configured maximum size",
        ));
    }
    if !server.config.privacy.allow_remote_source && looks_like_source(&state) {
        return Ok(ToolOutput::error(
            "PERMISSION_DENIED",
            "outbound requests carrying repository source are disabled by the privacy policy",
        ));
    }

    let mut request = DecisionRequest::new(task, state.clone());
    if let Some(options) = args.get("options").and_then(|o| o.as_array()) {
        let list: Vec<String> = options
            .iter()
            .filter_map(|o| o.as_str().map(|s| s.to_string()))
            .collect();
        request = request.with_options(list);
    }
    if let Some(hint) = args.get("hint").and_then(|h| h.as_str()) {
        request.hint = Some(hint.to_string());
    }

    let outcome = server.router.route(&request).await?;
    let text = format!(
        "OK | {} | choice={} | confidence={} | provider={}{}",
        task.question_name(),
        outcome.choice.clone().unwrap_or_else(|| "none".into()),
        outcome
            .confidence
            .map(|c| format!("{c:.2}"))
            .unwrap_or_else(|| "n/a".into()),
        outcome.provider,
        if outcome.cached { " | cached" } else { "" }
    );
    Ok(ToolOutput::ok(
        text,
        json!({
            "task": task.question_name(),
            "choice": outcome.choice,
            "confidence": outcome.confidence,
            "probability": outcome.probability,
            "provider": outcome.provider,
            "cached": outcome.cached,
            "input_tokens": outcome.input_tokens,
            "output_tokens": outcome.output_tokens,
            "escalated": outcome.escalated
        }),
    ))
}

fn looks_like_source(text: &str) -> bool {
    let symbols = ['{', '}', ';', '(', ')', '='];
    let hits = symbols.iter().filter(|s| text.contains(**s)).count();
    hits >= 3 || text.lines().count() > 12
}

/// The last excerpt delivered for a file, when the caller read it.
fn last_read(server: &Server, path: &str) -> Option<crate::patch::ReadBound> {
    let abs = devzymer_core::util::resolve_inside(&server.root, std::path::Path::new(path)).ok()?;
    let key =
        devzymer_core::util::relative_slash(&server.root, &abs).unwrap_or_else(|| path.to_string());
    let reads = server.state.reads.lock().ok()?;
    let record = reads.get(&key)?;
    Some(crate::patch::ReadBound {
        start_line: record.start_line,
        end_line: record.end_line,
        sha: record.sha.clone(),
    })
}

/// Locates a symbol by identifier, qualified name or plain name.
pub fn find_symbol(server: &Server, reference: &str) -> Result<Option<Symbol>> {
    if let Some(symbol) = server.store.symbol_by_id(reference)? {
        return Ok(Some(symbol));
    }
    let qualified = server.store.symbols_by_qualified_name(reference)?;
    if let Some(first) = qualified.into_iter().next() {
        return Ok(Some(first));
    }
    let by_name = server.store.symbols_by_name(reference)?;
    Ok(by_name.into_iter().next())
}

/// Resolves a repository relative path, refusing to escape the workspace.
pub fn safe_join(root: &Path, path: &str) -> Result<std::path::PathBuf> {
    devzymer_core::util::resolve_inside(root, Path::new(path))
}
