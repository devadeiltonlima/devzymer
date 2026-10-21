use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use devzymer_core::config::Config;
use devzymer_core::paths;
use devzymer_core::store::Store;
use devzymer_graph::build_graph;
use devzymer_indexer::index::{IndexOptions, Indexer};
use devzymer_indexer::scan::ScanOptions;
use devzymer_integrations::setup::{self, SetupReport};
use devzymer_integrations::{AgentKind, AgentOutcome, InstallScope};
use devzymer_tui::brand::{self, Caps, Mark, Task};

use crate::init_state::{self, Stage};
use crate::Ctx;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// As opções do comando de inicialização.
pub struct InitOptions {
    pub full: bool,
    pub no_integrate: bool,
    pub dry_run: bool,
    pub scope: Option<String>,
    pub server_only: bool,
    pub agents: Option<String>,
    pub yes: bool,
    pub no_agent_setup: bool,
    pub repair: bool,
}

fn scan_options(ctx: &Ctx) -> ScanOptions {
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
    scan_options
}

/// O que a inspeção do índice existente concluiu.
enum IndexHealth {
    Absent,
    Healthy,
    Unusable(String),
}

/// Looks at an existing index without changing it.
///
/// An index that cannot be opened, fails the SQLite check, has an
/// inconsistent full text index or belongs to another workspace is reported,
/// never silently rebuilt over.
fn preflight_index(root: &Path) -> IndexHealth {
    let db = paths::db_path(root);
    if !db.exists() {
        return IndexHealth::Absent;
    }
    let store = match Store::open(&db) {
        Ok(store) => store,
        Err(error) => return IndexHealth::Unusable(error.to_string()),
    };
    match store.quick_check() {
        Ok(result) if result == "ok" => {}
        Ok(result) => {
            return IndexHealth::Unusable(format!("verificação do SQLite: {result}"));
        }
        Err(error) => return IndexHealth::Unusable(error.to_string()),
    }
    if let Err(error) = store.fts_integrity_check() {
        return IndexHealth::Unusable(format!("índice lexical FTS5 inconsistente: {error}"));
    }
    IndexHealth::Healthy
}

/// Moves an unusable index aside. It is renamed, never deleted, so nothing is
/// lost if the diagnosis was wrong.
fn quarantine_index(root: &Path) -> Result<PathBuf> {
    let db = paths::db_path(root);
    let stamp = devzymer_core::util::now_millis();
    let name = db
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| paths::DB_FILE.to_string());
    let aside = db.with_file_name(format!("{name}.corrupt-{stamp}"));
    std::fs::rename(&db, &aside).map_err(|error| {
        anyhow::anyhow!(
            "não foi possível guardar o índice danificado ({error}). \
             Feche os agentes que usam este projeto e tente de novo"
        )
    })?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", db.display()));
        if side.exists() {
            let _ = std::fs::rename(&side, format!("{}{suffix}", aside.display()));
        }
    }
    Ok(aside)
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
        no_agent_setup,
        repair,
    } = options;
    let root = ctx.resolve_root();
    let workspace = devzymer_core::workspace::Workspace::detect(&root);
    let detections = devzymer_integrations::detect(&root);
    let scan_options = scan_options(ctx);
    // `--no-integrate` writes nothing for the agents, files included.
    let agent_files_enabled = !no_agent_setup && !no_integrate;
    let requested: Option<Vec<AgentKind>> =
        agents.as_deref().map(AgentKind::parse_list).transpose()?;

    if dry_run {
        let report = devzymer_indexer::scan::scan(&root, &scan_options)?;
        let planned = agent_files_enabled.then(|| {
            let hosts = requested.clone().unwrap_or_else(|| AgentKind::ALL.to_vec());
            setup::prepare(&root, &hosts, false)
        });
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
            "duration_ms": report.duration_ms,
            "agent_files": planned,
            "init_state": init_state::read(&root).map(|s| s.stage),
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
            match &planned {
                Some(plan) => {
                    println!("arquivos do agente (nada foi gravado):");
                    print_items_plain(plan);
                }
                None => println!("arquivos do agente: desativados por --no-agent-setup"),
            }
        }
        return Ok(());
    }

    let caps = brand::detect();
    let fancy = caps.color && !ctx.json;
    let interactive = devzymer_tui::is_interactive()
        || std::env::var("DEVZYMER_FORCE_TUI").is_ok_and(|value| !value.is_empty());
    let uses_selection_screen = agents.is_none() && !yes && !no_integrate && interactive;
    if fancy && !uses_selection_screen {
        brand::print_banner(&caps, true, VERSION)?;
    }

    let scope = resolve_scope(ctx, scope.as_deref())?;
    let (selected, explicit_selection) = resolve_agents(
        ctx,
        &detections,
        agents.as_deref(),
        yes,
        no_integrate,
        scope,
    )?;

    // An index that cannot be trusted is never overwritten as a side effect.
    // `--repair` is the explicit authorization to move it aside and rebuild.
    let mut quarantined: Option<PathBuf> = None;
    if let IndexHealth::Unusable(reason) = preflight_index(&root) {
        if !repair {
            bail!(
                "o índice existente em {} não pôde ser usado: {reason}.\n\
                 Nada foi alterado. Para guardar esse índice ao lado e reconstruir um novo, \
                 rode `devzymer init --repair`.",
                paths::db_path(&root).display()
            );
        }
        quarantined = Some(quarantine_index(&root)?);
    }

    if fancy {
        if uses_selection_screen {
            brand::print_banner(&caps, false, VERSION)?;
        }
        brand::heading(&caps, "Preparando o seu projeto")?;
        let languages = if workspace.languages.is_empty() {
            "nenhuma linguagem reconhecida".to_string()
        } else {
            workspace.languages.join(", ")
        };
        brand::line(&caps, Mark::Ok, "Ambiente detectado", &languages)?;
        if let Some(aside) = &quarantined {
            brand::line(
                &caps,
                Mark::Warn,
                "Índice anterior preservado",
                &aside.display().to_string(),
            )?;
        }
    } else if !ctx.json {
        if let Some(aside) = &quarantined {
            println!("índice anterior preservado em {}", aside.display());
        }
    }

    init_state::write(&root, Stage::Indexing, None);

    let db_path = paths::db_path(&root);
    let store = Store::open(&db_path)?;
    // The index is rebuilt in full when its parser is not the current one, and
    // for a new index. Only then is the parser version recorded as current:
    // every symbol was produced by this parser. An older index is never
    // relabeled without being reparsed.
    let had_files = store.stats()?.files > 0;
    let parser_was_stale = store.parser_version_changed();
    let indexer = Indexer::new(&store, &root);
    let options = IndexOptions {
        root: root.clone(),
        scan: scan_options,
        force_full: full || parser_was_stale,
        chunk_size: 256,
    };
    let index_report = timed(
        &caps,
        fancy,
        "Indexando o repositório",
        || indexer.index(&options).map_err(anyhow::Error::from),
        |report| {
            format!(
                "{} arquivos · {} símbolos",
                report.files_indexed + report.files_unchanged,
                report.symbols
            )
        },
    )?;
    let mut parser_note: Option<String> = None;
    if parser_was_stale && had_files && !index_report.errors.is_empty() {
        // Files that failed to parse keep symbols from the old parser, so the
        // index cannot claim to be current.
        store.set_meta("indexed_parser_version", "incomplete")?;
        parser_note = Some(format!(
            "{} arquivo(s) não puderam ser relidos; o índice não foi marcado como atual",
            index_report.errors.len()
        ));
    }

    let graph_report = timed(
        &caps,
        fancy,
        "Montando o grafo de chamadas",
        || build_graph(&store).map_err(anyhow::Error::from),
        |graph| format!("{} relações", graph.relations),
    )?;
    drop(store);
    init_state::write(&root, Stage::Indexed, parser_note.as_deref());

    let verifying = fancy.then(|| Task::start(&caps, "Verificando o índice"));
    let verification = verify(&root, &ctx.config).await?;
    if let Some(task) = verifying {
        let detail = format!(
            "banco {} · fts5 {} · busca {} · mcp {} · parser {}",
            verdict(verification.database_ok),
            verdict(verification.fts_ok),
            verdict(verification.search_ok),
            verdict(verification.mcp_ok),
            verdict(verification.parser_current)
        );
        if verification.index_healthy() {
            task.finish(&detail);
        } else {
            task.fail(&detail);
        }
    }

    let executable = std::env::current_exe()?;
    let mut outcomes: Vec<AgentOutcome> = Vec::new();
    let mut agent_files: Option<SetupReport> = None;
    let mut skipped_reason: Option<&'static str> = None;

    if verification.index_healthy() {
        init_state::write(&root, Stage::Verified, parser_note.as_deref());
        // The project files go first, so their report is the accurate one: the
        // host installers then find the instructions already in place.
        if agent_files_enabled {
            let hosts = if explicit_selection {
                selected.clone()
            } else {
                AgentKind::ALL.to_vec()
            };
            let task = fancy.then(|| Task::start(&caps, "Preparando instruções e skill"));
            let report = setup::prepare(&root, &hosts, true);
            if let Some(task) = task {
                if report.has_blockers() {
                    task.fail("há arquivos que não foram alterados");
                } else {
                    task.finish(&format!("{} item(ns)", report.items.len()));
                }
            }
            agent_files = Some(report);
        }
        if !no_integrate && !selected.is_empty() {
            if !ctx.json && !fancy {
                println!("Instalando o devzymer...");
                println!("[✓] Índice do repositório verificado");
            }
            for agent in &selected {
                let task =
                    fancy.then(|| Task::start(&caps, &format!("Integrando {}", agent.display())));
                let outcome = install_one(
                    ctx,
                    &root,
                    *agent,
                    scope,
                    &executable,
                    server_only,
                    agent_files_enabled,
                    fancy,
                )?;
                if let Some(task) = task {
                    if outcome.ok {
                        task.finish(&outcome.detail);
                    } else {
                        task.fail(&outcome.detail);
                    }
                }
                outcomes.push(outcome);
            }
        }
    } else {
        skipped_reason = Some(
            "o índice não passou na verificação; o registro dos agentes e os arquivos do \
             projeto não foram gravados",
        );
    }

    let integrations_ok = outcomes.iter().all(|outcome| outcome.ok);
    let files_ok = agent_files.as_ref().is_none_or(|r| !r.has_blockers());
    let healthy =
        verification.index_healthy() && verification.parser_current && integrations_ok && files_ok;
    if verification.index_healthy() {
        init_state::write(
            &root,
            if healthy {
                Stage::Complete
            } else {
                Stage::AgentFiles
            },
            parser_note.as_deref(),
        );
    }

    let payload = serde_json::json!({
        "root": root.display().to_string(),
        "index": index_report,
        "graph": graph_report,
        "agents": detections,
        "installed": outcomes,
        "agent_files": agent_files,
        "verification": verification,
        "quarantined_index": quarantined.as_ref().map(|p| p.display().to_string()),
        "parser_note": parser_note,
        "skipped": skipped_reason,
        "stage": init_state::read(&root).map(|s| s.stage),
    });

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if fancy {
        print_summary(
            &caps,
            &index_report,
            &outcomes,
            agent_files.as_ref(),
            skipped_reason,
            parser_note.as_deref(),
            healthy,
        )?;
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
    if let Some(report) = &agent_files {
        println!("arquivos do agente:");
        print_items_plain(report);
        print_host_steps_plain(report);
    }
    if let Some(reason) = skipped_reason {
        println!("aviso: {reason}");
    }
    if let Some(note) = &parser_note {
        println!("aviso: {note}");
    }
    println!(
        "validado: busca={} | banco={} | fts5={} | mcp={} | parser={}",
        verification.search_ok,
        verification.database_ok,
        verification.fts_ok,
        verification.mcp_ok,
        verification.parser_current
    );
    if !index_report.errors.is_empty() {
        println!("erros por arquivo: {}", index_report.errors.len());
        for err in index_report.errors.iter().take(5) {
            println!("  {err}");
        }
    }
    Ok(())
}

fn timed<T>(
    caps: &Caps,
    fancy: bool,
    label: &str,
    work: impl FnOnce() -> Result<T>,
    detail: impl FnOnce(&T) -> String,
) -> Result<T> {
    if !fancy {
        return work();
    }
    let task = Task::start(caps, label);
    match work() {
        Ok(value) => {
            task.finish(&detail(&value));
            Ok(value)
        }
        Err(error) => {
            task.fail(&error.to_string());
            Err(error)
        }
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "falhou"
    }
}

fn action_label(action: &str) -> &'static str {
    match action {
        "created" => "criado",
        "appended" => "bloco acrescentado",
        "updated" => "atualizado",
        "unchanged" => "sem mudanças",
        "partial" => "parcial (arquivos seus mantidos)",
        "partial_updated" => "atualizado em parte (arquivos seus mantidos)",
        "skipped" => "não alterado",
        _ => "—",
    }
}

fn print_items_plain(report: &SetupReport) {
    for item in &report.items {
        println!(
            "  {} {}: {}",
            if item.action == "skipped" {
                "[✗]"
            } else {
                "[✓]"
            },
            item.path,
            action_label(&item.action)
        );
        if item.action == "skipped" {
            println!("      {}", item.detail);
        }
    }
    for warning in &report.warnings {
        println!("  aviso: {warning}");
    }
}

fn print_host_steps_plain(report: &SetupReport) {
    for host in &report.hosts {
        for step in &host.next_steps {
            println!("  {}: {step}", host.agent.display());
        }
    }
}

fn print_summary(
    caps: &Caps,
    index_report: &devzymer_indexer::index::IndexReport,
    outcomes: &[AgentOutcome],
    agent_files: Option<&SetupReport>,
    skipped_reason: Option<&str>,
    parser_note: Option<&str>,
    healthy: bool,
) -> Result<()> {
    for outcome in outcomes.iter().filter(|outcome| !outcome.ok) {
        for change in &outcome.changes {
            println!("      {} -> {}", change.file, change.detail);
        }
    }
    if let Some(report) = agent_files {
        for item in &report.items {
            let mark = if item.action == "skipped" {
                Mark::Warn
            } else {
                Mark::Ok
            };
            brand::line(caps, mark, &item.path, action_label(&item.action))?;
            if item.action == "skipped" {
                println!("      {}", item.detail);
            }
        }
        for warning in &report.warnings {
            brand::line(caps, Mark::Warn, "Atenção", warning)?;
        }
    }
    if let Some(reason) = skipped_reason {
        brand::line(caps, Mark::Warn, "Etapas não executadas", reason)?;
    }
    if let Some(note) = parser_note {
        brand::line(caps, Mark::Warn, "Índice", note)?;
    }
    if !index_report.errors.is_empty() {
        brand::line(
            caps,
            Mark::Warn,
            "Arquivos com erro de leitura",
            &format!("{} (mostrando até 3)", index_report.errors.len()),
        )?;
        for error in index_report.errors.iter().take(3) {
            println!("      {error}");
        }
    }
    println!();
    if !healthy {
        brand::line(
            caps,
            Mark::Warn,
            "Instalado com avisos",
            "rode `devzymer doctor` para ver o que falhou",
        )?;
        println!();
        return Ok(());
    }
    brand::heading(caps, "Tudo pronto. O devzymer está ativo neste projeto.")?;
    println!("  Próximos passos");
    let mut step = 1;
    let restart = agent_files.is_some_and(|r| r.changed())
        || outcomes
            .iter()
            .any(|o| o.changes.iter().any(|c| c.action == "updated"));
    if restart {
        println!(
            "    {step}. Abra uma nova sessão do seu agente neste projeto (Claude Code, Codex ou OpenCode)"
        );
    } else {
        println!("    {step}. Abra o seu agente neste projeto (Claude Code, Codex ou OpenCode)");
    }
    step += 1;
    if let Some(report) = agent_files {
        for host in &report.hosts {
            if !host.mcp_registered {
                println!(
                    "    {step}. {}: o servidor MCP ainda não está registrado: devzymer integrations add {}",
                    host.agent.display(),
                    host.agent.key()
                );
                step += 1;
            }
        }
    }
    println!("    {step}. Peça algo como: \"encontre onde a sessão do usuário é validada\"");
    step += 1;
    println!("    {step}. Confira o estado quando quiser: devzymer status");
    println!();
    Ok(())
}

/// Instala um agente e devolve o resultado, com a falha isolada.
#[allow(clippy::too_many_arguments)]
fn install_one(
    ctx: &Ctx,
    root: &Path,
    agent: AgentKind,
    scope: InstallScope,
    executable: &Path,
    server_only: bool,
    rules: bool,
    quiet: bool,
) -> Result<AgentOutcome> {
    let outcome = devzymer_integrations::install_with_rules(
        root,
        &ctx.config,
        &[agent],
        scope,
        executable,
        server_only,
        rules,
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
    if !ctx.json && !quiet {
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

/// Decide os agentes em que o servidor MCP é registrado.
///
/// A linha de comando vence. Sem ela, a interface aparece quando há terminal
/// interativo. Sem terminal, o comportamento histórico é mantido: apenas o
/// Claude Code, com um aviso explícito. O segundo valor diz se a escolha foi
/// explícita; sem escolha, os arquivos do projeto cobrem os três agentes.
fn resolve_agents(
    ctx: &Ctx,
    detections: &[devzymer_integrations::AgentDetection],
    agents: Option<&str>,
    yes: bool,
    no_integrate: bool,
    scope: InstallScope,
) -> Result<(Vec<AgentKind>, bool)> {
    if no_integrate {
        return Ok((Vec::new(), false));
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
        return Ok((parsed, true));
    }
    let interactive = devzymer_tui::is_interactive()
        || std::env::var("DEVZYMER_FORCE_TUI").is_ok_and(|value| !value.is_empty());
    if yes || !interactive {
        if !ctx.json {
            println!(
                "aviso: sem terminal interativo, o servidor MCP é registrado apenas no Claude Code. \
                 Use `--agents claude,codex,opencode` para registrar nos demais."
            );
        }
        return Ok((vec![AgentKind::Claude], false));
    }

    let mut selection = devzymer_tui::Selection::new(detections);
    match devzymer_tui::run(&mut selection) {
        Ok(devzymer_tui::Outcome::Confirmed(choice)) => {
            let _ = scope;
            Ok((choice.agents, true))
        }
        Ok(devzymer_tui::Outcome::Cancelled) => {
            bail!("instalação cancelada. Nada foi alterado.")
        }
        Err(error) => {
            if !ctx.json {
                println!(
                    "aviso: a interface não pôde ser aberta ({error}). \
                     O servidor MCP é registrado apenas no Claude Code. Use `--agents` para escolher os agentes."
                );
            }
            Ok((vec![AgentKind::Claude], false))
        }
    }
}

#[derive(serde::Serialize)]
pub struct Verification {
    pub search_ok: bool,
    pub database_ok: bool,
    pub fts_ok: bool,
    pub mcp_ok: bool,
    /// True when every symbol in the index was produced by the current parser.
    pub parser_current: bool,
    pub detail: Vec<String>,
}

impl Verification {
    /// True when the index can be served to an agent. A parser version that
    /// is not current is reported as a warning, not a reason to withhold the
    /// integration: the routing hooks already treat such an index as stale.
    pub fn index_healthy(&self) -> bool {
        self.search_ok && self.database_ok && self.fts_ok && self.mcp_ok
    }
}

async fn verify(root: &Path, config: &Config) -> Result<Verification> {
    let store = Store::open(&paths::db_path(root))?;
    let integrity = store.integrity_check()?;
    let database_ok = integrity == "ok";
    let fts = store.fts_integrity_check();
    let fts_ok = fts.is_ok();
    let parser_current = !store.parser_version_changed();

    let mut detail = Vec::new();
    if let Err(error) = &fts {
        detail.push(format!("fts5: {error}"));
    }
    let stats = store.stats()?;
    let mut search_ok = false;
    if stats.symbols == 0 {
        // Nothing to probe: a project without indexable code is not a failure.
        search_ok = true;
        detail.push("sem símbolos para sondar a busca".to_string());
    } else {
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
        fts_ok,
        mcp_ok,
        parser_current,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-init-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub fn validate_session(token: &str) -> bool {\n    !token.is_empty()\n}\n",
        )
        .unwrap();
        devzymer_core::util::canonical_display(&dir)
    }

    fn ctx(root: &Path) -> Ctx {
        Ctx {
            root: root.to_path_buf(),
            config: Config::default(),
            json: true,
        }
    }

    fn options(agents: &str) -> InitOptions {
        InitOptions {
            full: false,
            no_integrate: false,
            dry_run: false,
            scope: None,
            server_only: false,
            agents: Some(agents.to_string()),
            yes: true,
            no_agent_setup: false,
            repair: false,
        }
    }

    fn parser_marker(root: &Path) -> Option<String> {
        Store::open(&paths::db_path(root))
            .unwrap()
            .indexed_parser_version()
    }

    const TRACKED: [&str; 7] = [
        "CLAUDE.md",
        "AGENTS.md",
        ".claude/skills/devzymer/SKILL.md",
        ".claude/skills/devzymer/.devzymer-manifest.json",
        ".agents/skills/devzymer/SKILL.md",
        ".claude/settings.json",
        ".mcp.json",
    ];

    fn snapshot(root: &Path) -> Vec<Vec<u8>> {
        TRACKED
            .iter()
            .map(|p| std::fs::read(root.join(p)).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn init_records_the_parser_version_of_a_new_index() {
        let root = project("fresh");
        run(&ctx(&root), options("claude")).await.unwrap();
        assert_eq!(
            parser_marker(&root).as_deref(),
            Some(devzymer_core::store::PARSER_VERSION)
        );
        assert_eq!(
            init_state::read(&root).map(|s| s.stage),
            Some(Stage::Complete)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_old_index_is_reparsed_before_it_is_marked_current() {
        let root = project("stale");
        run(&ctx(&root), options("claude")).await.unwrap();
        {
            let store = Store::open(&paths::db_path(&root)).unwrap();
            store
                .set_meta("indexed_parser_version", "parser-antigo")
                .unwrap();
        }
        run(&ctx(&root), options("claude")).await.unwrap();
        assert_eq!(
            parser_marker(&root).as_deref(),
            Some(devzymer_core::store::PARSER_VERSION)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_agent_files_cover_every_host_by_default() {
        let root = project("hosts");
        let mut opts = options("claude");
        opts.agents = None;
        run(&ctx(&root), opts).await.unwrap();
        for file in [
            "CLAUDE.md",
            "AGENTS.md",
            ".claude/skills/devzymer/SKILL.md",
            ".agents/skills/devzymer/SKILL.md",
        ] {
            assert!(root.join(file).exists(), "{file}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_second_init_changes_no_agent_file() {
        let root = project("twice");
        std::fs::write(root.join("AGENTS.md"), "# Meu projeto\n\nUse pnpm.\n").unwrap();
        run(&ctx(&root), options("claude,codex,opencode"))
            .await
            .unwrap();
        let before = snapshot(&root);
        run(&ctx(&root), options("claude,codex,opencode"))
            .await
            .unwrap();
        assert_eq!(before, snapshot(&root));
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.starts_with("# Meu projeto\n\nUse pnpm.\n"));
        assert_eq!(agents.matches("<!-- devzymer:start -->").count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn no_agent_setup_writes_no_instructions_or_skills() {
        let root = project("no-setup");
        let mut opts = options("claude");
        opts.no_agent_setup = true;
        run(&ctx(&root), opts).await.unwrap();
        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join(".claude/skills").exists());
        assert!(!root.join("AGENTS.md").exists());
        assert!(root.join(".mcp.json").exists(), "o registro MCP segue");
        assert!(paths::db_path(&root).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn no_integrate_writes_nothing_for_the_agents() {
        let root = project("no-integrate");
        let mut opts = options("claude");
        opts.no_integrate = true;
        run(&ctx(&root), opts).await.unwrap();
        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join(".mcp.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_dry_run_leaves_the_project_untouched() {
        let root = project("dry");
        let mut opts = options("claude");
        opts.dry_run = true;
        run(&ctx(&root), opts).await.unwrap();
        assert!(!root.join(".devzymer").exists());
        assert!(!root.join("CLAUDE.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_damaged_index_is_not_replaced_without_repair() {
        let root = project("damaged");
        run(&ctx(&root), options("claude")).await.unwrap();
        let db = paths::db_path(&root);
        std::fs::write(
            &db,
            b"isto nao e um banco sqlite, mas e grande o bastante para parecer um",
        )
        .unwrap();
        let before = std::fs::read(&db).unwrap();

        let error = run(&ctx(&root), options("claude")).await.unwrap_err();
        assert!(error.to_string().contains("--repair"), "{error}");
        assert_eq!(
            std::fs::read(&db).unwrap(),
            before,
            "o índice não pode mudar"
        );

        let mut repair = options("claude");
        repair.repair = true;
        run(&ctx(&root), repair).await.unwrap();
        let aside: Vec<_> = std::fs::read_dir(db.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "o índice danificado fica preservado");
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), before);
        assert!(matches!(preflight_index(&root), IndexHealth::Healthy));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_index_of_another_workspace_is_refused() {
        let first = project("owner-a");
        let second = project("owner-b");
        run(&ctx(&first), options("claude")).await.unwrap();
        std::fs::create_dir_all(second.join(".devzymer")).unwrap();
        std::fs::copy(paths::db_path(&first), paths::db_path(&second)).unwrap();

        let error = run(&ctx(&second), options("claude")).await.unwrap_err();
        assert!(error.to_string().contains("outro workspace"), "{error}");
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[tokio::test]
    async fn a_project_without_symbols_still_initializes() {
        let root = project("empty");
        std::fs::remove_file(root.join("src/lib.rs")).unwrap();
        run(&ctx(&root), options("claude")).await.unwrap();
        assert_eq!(
            init_state::read(&root).map(|s| s.stage),
            Some(Stage::Complete)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_user_edited_skill_survives_a_new_init() {
        let root = project("edited");
        run(&ctx(&root), options("claude")).await.unwrap();
        let skill = root.join(".claude/skills/devzymer/SKILL.md");
        let edited = format!("{}\nMeu fluxo.\n", std::fs::read_to_string(&skill).unwrap());
        std::fs::write(&skill, &edited).unwrap();
        run(&ctx(&root), options("claude")).await.unwrap();
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn the_skill_only_cites_tools_the_server_exposes() {
        let root = project("tools");
        let store = Store::open(&paths::db_path(&root)).unwrap();
        let server =
            devzymer_mcp::Server::with_store(root.clone(), Config::default(), store).unwrap();
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
            .await
            .unwrap();
        for tool in devzymer_integrations::skill::REQUIRED_TOOLS {
            assert!(response.contains(&format!("\"{tool}\"")), "{tool}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
