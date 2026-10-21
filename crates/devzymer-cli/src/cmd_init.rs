use std::path::Path;

use anyhow::{bail, Result};
use devzymer_core::config::Config;
use devzymer_core::paths;
use devzymer_core::store::Store;
use devzymer_graph::build_graph;
use devzymer_indexer::index::{IndexOptions, Indexer};
use devzymer_indexer::scan::ScanOptions;
use devzymer_integrations::{AgentKind, AgentOutcome, InstallScope};

use crate::Ctx;

/// As opções do comando de inicialização.
pub struct InitOptions {
    pub full: bool,
    pub no_integrate: bool,
    pub dry_run: bool,
    pub scope: Option<String>,
    pub server_only: bool,
    pub agents: Option<String>,
    pub yes: bool,
}

pub async fn run(ctx: &Ctx, options: InitOptions) -> Result<()> {
    let InitOptions {
        full,
        no_integrate,
        dry_run,
        scope,
        server_only,
        agents,
        yes,
    } = options;
    let root = ctx.resolve_root();
    let workspace = devzymer_core::workspace::Workspace::detect(&root);
    let detections = devzymer_integrations::detect(&root);

    let mut scan_options = ScanOptions {
        exclude: ctx.config.index.exclude.clone(),
        exclude_secret_files: ctx.config.privacy.exclude_secret_files,
        max_file_size: ctx.config.index.max_file_size_bytes,
        follow_symlinks: ctx.config.index.follow_symlinks,
        respect_gitignore: ctx.config.index.respect_gitignore,
        include_hidden: ctx.config.index.include_hidden,
        languages: Vec::new(),
        max_workers: ctx.config.effective_workers(),
        max_duration_ms: None,
    };
    if !ctx.config.index.languages.is_empty() {
        scan_options.languages = ctx
            .config
            .index
            .languages
            .iter()
            .filter_map(|l| devzymer_core::model::Language::from_display(l))
            .collect();
    }

    if dry_run {
        let report = devzymer_indexer::scan::scan(&root, &scan_options)?;
        let payload = serde_json::json!({
            "root": root.display().to_string(),
            "languages": workspace.languages,
            "manifests": workspace.manifests,
            "nested_repos": workspace.nested_repos,
            "claude_cli": workspace.claude.cli_installed,
            "codegraph": workspace.codegraph,
            "agents": detections,
            "files": report.files.len(),
            "skipped": report.skipped.len(),
            "duration_ms": report.duration_ms
        });
        if ctx.json {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        } else {
            println!("raiz: {}", root.display());
            println!("linguagens: {}", workspace.languages.join(", "));
            println!("manifestos: {}", workspace.manifests.join(", "));
            println!("arquivos indexáveis: {}", report.files.len());
            println!("arquivos ignorados: {}", report.skipped.len());
            println!("codegraph detectado: {}", workspace.codegraph.present);
            for agent in &detections {
                println!(
                    "agente: {} -> {}",
                    agent.agent.display(),
                    agent.status.display()
                );
            }
        }
        return Ok(());
    }

    let scope = resolve_scope(ctx, scope.as_deref())?;
    let selected = resolve_agents(
        ctx,
        &detections,
        agents.as_deref(),
        yes,
        no_integrate,
        scope,
    )?;

    let db_path = paths::db_path(&root);
    let store = Store::open(&db_path)?;
    let indexer = Indexer::new(&store, &root);
    let options = IndexOptions {
        root: root.clone(),
        scan: scan_options,
        force_full: full,
        chunk_size: 256,
    };
    let index_report = indexer.index(&options)?;

    let graph_report = build_graph(&store)?;

    let executable = std::env::current_exe()?;
    let mut outcomes: Vec<AgentOutcome> = Vec::new();
    if !no_integrate && !selected.is_empty() {
        if !ctx.json {
            println!("Instalando o devzymer...");
            println!("[✓] Índice do repositório verificado");
        }
        for agent in &selected {
            let outcome = install_one(ctx, &root, *agent, scope, &executable, server_only)?;
            outcomes.push(outcome);
        }
    }

    let verification = verify(&root, &ctx.config).await?;

    let payload = serde_json::json!({
        "root": root.display().to_string(),
        "index": index_report,
        "graph": graph_report,
        "agents": detections,
        "installed": outcomes,
        "verification": verification
    });

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!("implementado: índice criado em {}", db_path.display());
    println!(
        "  arquivos: {} indexados, {} inalterados, {} removidos, {} ignorados",
        index_report.files_indexed,
        index_report.files_unchanged,
        index_report.files_removed,
        index_report.files_skipped
    );
    println!("  símbolos: {}", index_report.symbols);
    println!(
        "  grafo: {} relações ({} chamadas, {} importações)",
        graph_report.relations, graph_report.calls, graph_report.imports
    );
    for outcome in &outcomes {
        println!(
            "  {} {}: {}",
            if outcome.ok { "[✓]" } else { "[✗]" },
            outcome.agent.display(),
            outcome.detail
        );
        for change in &outcome.changes {
            println!("      {} -> {}", change.file, change.detail);
        }
    }
    println!(
        "validado: busca={} | banco={} | mcp={}",
        verification.search_ok, verification.database_ok, verification.mcp_ok
    );
    if !index_report.errors.is_empty() {
        println!("erros por arquivo: {}", index_report.errors.len());
        for err in index_report.errors.iter().take(5) {
            println!("  {err}");
        }
    }
    Ok(())
}

/// Instala um agente e devolve o resultado, com a falha isolada.
fn install_one(
    ctx: &Ctx,
    root: &Path,
    agent: AgentKind,
    scope: InstallScope,
    executable: &Path,
    server_only: bool,
) -> Result<AgentOutcome> {
    let outcome = devzymer_integrations::install(
        root,
        &ctx.config,
        &[agent],
        scope,
        executable,
        server_only,
    )?
    .into_iter()
    .next()
    .unwrap_or(AgentOutcome {
        agent,
        scope,
        ok: false,
        verified: false,
        detail: "nenhum resultado".to_string(),
        changes: Vec::new(),
    });
    if !ctx.json {
        println!(
            "[{}] {}",
            if outcome.ok { "✓" } else { "✗" },
            outcome.agent.display()
        );
    }
    Ok(outcome)
}

/// Decide o escopo: a linha de comando vence, depois a interface, depois a
/// configuração.
fn resolve_scope(ctx: &Ctx, scope: Option<&str>) -> Result<InstallScope> {
    if let Some(scope) = scope {
        return InstallScope::parse(scope).ok_or_else(|| {
            anyhow::anyhow!("escopo desconhecido '{scope}'. Use project, local ou user.")
        });
    }
    Ok(InstallScope::parse(&ctx.config.mcp.install_scope).unwrap_or(InstallScope::Project))
}

/// Decide os agentes a integrar.
///
/// A linha de comando vence. Sem ela, a interface aparece quando há terminal
/// interativo. Sem terminal, o comportamento histórico é mantido: apenas o
/// Claude Code, com um aviso explícito.
fn resolve_agents(
    ctx: &Ctx,
    detections: &[devzymer_integrations::AgentDetection],
    agents: Option<&str>,
    yes: bool,
    no_integrate: bool,
    scope: InstallScope,
) -> Result<Vec<AgentKind>> {
    if no_integrate {
        return Ok(Vec::new());
    }
    if let Some(list) = agents {
        let parsed = AgentKind::parse_list(list)?;
        for agent in &parsed {
            let detection = detections.iter().find(|d| d.agent == *agent);
            if let Some(detection) = detection {
                if !detection.is_available() && !ctx.json {
                    println!(
                        "aviso: {} não foi encontrado neste ambiente. A configuração será gravada mesmo assim.",
                        agent.display()
                    );
                }
            }
        }
        return Ok(parsed);
    }
    let interactive = devzymer_tui::is_interactive()
        || std::env::var("DEVZYMER_FORCE_TUI").is_ok_and(|value| !value.is_empty());
    if yes || !interactive {
        if !ctx.json {
            println!(
                "aviso: sem terminal interativo, apenas o Claude Code é integrado. \
                 Use `--agents claude,codex,opencode` para escolher os agentes."
            );
        }
        return Ok(vec![AgentKind::Claude]);
    }

    let mut selection = devzymer_tui::Selection::new(detections);
    match devzymer_tui::run(&mut selection) {
        Ok(devzymer_tui::Outcome::Confirmed(choice)) => {
            let _ = scope;
            Ok(choice.agents)
        }
        Ok(devzymer_tui::Outcome::Cancelled) => {
            bail!("instalação cancelada. Nada foi alterado.")
        }
        Err(error) => {
            if !ctx.json {
                println!(
                    "aviso: a interface não pôde ser aberta ({error}). \
                     Apenas o Claude Code é integrado. Use `--agents` para escolher os agentes."
                );
            }
            Ok(vec![AgentKind::Claude])
        }
    }
}

#[derive(serde::Serialize)]
pub struct Verification {
    pub search_ok: bool,
    pub database_ok: bool,
    pub mcp_ok: bool,
    pub detail: Vec<String>,
}

async fn verify(root: &Path, config: &Config) -> Result<Verification> {
    let store = Store::open(&paths::db_path(root))?;
    let integrity = store.integrity_check()?;
    let database_ok = integrity == "ok";

    let mut search_ok = false;
    let mut detail = Vec::new();
    let stats = store.stats()?;
    if stats.symbols > 0 {
        let resolver = devzymer_search::Resolver::new(&store, config);
        let mut probe = devzymer_search::resolve::ResolveQuery::new("");
        probe.limit = 1;
        if let Some(symbol) = store.all_symbols()?.first() {
            probe.query = symbol.name.clone();
            search_ok = !resolver.resolve(&probe)?.results.is_empty();
            detail.push(format!("sonda de busca: {}", symbol.name));
        }
    }

    let server = devzymer_mcp::Server::with_store(root.to_path_buf(), config.clone(), store)?;
    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
    let response = server.handle_line(initialize).await;
    let mcp_ok = response
        .as_deref()
        .map(|r| r.contains("\"result\""))
        .unwrap_or(false);
    detail.push("handshake MCP verificado".to_string());

    Ok(Verification {
        search_ok,
        database_ok,
        mcp_ok,
        detail,
    })
}

pub async fn sync(ctx: &Ctx, full: bool, no_graph: bool) -> Result<()> {
    let root = ctx.resolve_root();
    let db_path = paths::db_path(&root);
    if !db_path.exists() {
        bail!(
            "índice não encontrado em {}. Execute `devzymer init` primeiro.",
            db_path.display()
        );
    }
    let store = Store::open(&db_path)?;
    let options = ScanOptions {
        exclude: ctx.config.index.exclude.clone(),
        exclude_secret_files: ctx.config.privacy.exclude_secret_files,
        max_file_size: ctx.config.index.max_file_size_bytes,
        follow_symlinks: ctx.config.index.follow_symlinks,
        respect_gitignore: ctx.config.index.respect_gitignore,
        include_hidden: ctx.config.index.include_hidden,
        languages: Vec::new(),
        max_workers: ctx.config.effective_workers(),
        max_duration_ms: None,
    };

    let (report, graph_report) = if full {
        let indexer = Indexer::new(&store, &root);
        let index_options = IndexOptions {
            root: root.clone(),
            scan: options,
            force_full: true,
            chunk_size: 256,
        };
        let index_report = indexer.index(&index_options)?;
        let graph = if no_graph {
            None
        } else {
            Some(build_graph(&store)?)
        };
        let sync_report = devzymer_indexer::sync::SyncReport {
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
        (sync_report, graph)
    } else {
        let sync_report = devzymer_indexer::sync::reconcile(&store, &root, &options)?;
        let graph = if no_graph {
            None
        } else {
            Some(build_graph(&store)?)
        };
        (sync_report, graph)
    };

    if ctx.json {
        let payload = serde_json::json!({ "sync": report, "graph": graph_report });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "sincronizado: +{} ~{} -{} | {} símbolos | {} ms{}",
        report.added.len(),
        report.modified.len(),
        report.removed.len(),
        report.symbols,
        report.duration_ms,
        if report.truncated { " | truncado" } else { "" }
    );
    if let Some(graph) = graph_report {
        println!("grafo: {} relações", graph.relations);
    }
    for error in report.errors.iter().take(5) {
        println!("erro: {error}");
    }
    Ok(())
}
