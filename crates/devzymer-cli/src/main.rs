mod cmd_bench;
mod cmd_config;
mod cmd_doctor;
mod cmd_hook;
mod cmd_init;
mod cmd_integrations;
mod cmd_maintain;
mod cmd_migrate;
mod cmd_routing;
mod cmd_serve;
mod cmd_status;
mod cmd_update;
mod cmd_watch;
mod init_state;
mod routing_hook;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use devzymer_core::config::Config;
use devzymer_core::paths;

#[derive(Parser)]
#[command(
    name = "devzymer",
    version,
    about = "devzymer: Less Tokens. More Engineering.",
    long_about = "devzymer indexa o repositório localmente e expõe recuperação determinística \
                  para agentes de código através de MCP, reduzindo tokens de entrada e saída."
)]
struct Cli {
    /// Raiz do workspace a operar.
    #[arg(long, global = true, value_name = "DIR")]
    root: Option<PathBuf>,

    /// Saída em JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Saída detalhada.
    #[arg(long, short, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Inicializa o índice, o grafo, as instruções e a skill do projeto e a integração com os agentes.
    Init {
        /// Reindexa tudo, ignorando o cache incremental.
        #[arg(long)]
        full: bool,
        /// Não registra o servidor MCP nem grava nada para os agentes (arquivos do
        /// projeto incluídos).
        #[arg(long)]
        no_integrate: bool,
        /// Apenas detecta o ambiente, sem escrever nada.
        #[arg(long)]
        dry_run: bool,
        /// Escopo da integração: project, local ou user.
        /// `user` registra o servidor em todos os projetos e em todos os hosts,
        /// incluindo a extensão do VS Code, sem aprovação por projeto.
        #[arg(long, value_name = "ESCOPO")]
        scope: Option<String>,
        /// Registra apenas o servidor MCP, sem as regras nem os hooks.
        #[arg(long)]
        server_only: bool,
        /// Agentes a integrar, separados por vírgula.
        /// Valores aceitos: claude, codex, opencode.
        #[arg(long, value_name = "LIST")]
        agents: Option<String>,
        /// Não abre a interface interativa.
        #[arg(long)]
        yes: bool,
        /// Não grava CLAUDE.md, AGENTS.md nem a skill do projeto. O registro do
        /// servidor MCP e os hooks seguem as outras opções.
        #[arg(long)]
        no_agent_setup: bool,
        /// Reaplica os arquivos do agente e, se o índice estiver danificado,
        /// guarda-o ao lado (sem apagar) e reconstrói um novo.
        #[arg(long)]
        repair: bool,
    },
    /// Mostra cobertura, atualidade e estado da integração.
    Status,
    /// Sincroniza os arquivos alterados.
    Sync {
        /// Reindexa tudo.
        #[arg(long)]
        full: bool,
        /// Não reconstrói o grafo de dependências.
        #[arg(long)]
        no_graph: bool,
    },
    /// Diagnostica instalação, banco, parsers e integrações.
    Doctor,
    /// Mantem o indice enxuto: checkpoint, espaco livre e cache.
    Maintain {
        /// Devolve as paginas livres ao arquivo.
        #[arg(long)]
        vacuum: bool,
    },
    /// Gerencia as integrações com os agentes de código.
    Integrations {
        #[command(subcommand)]
        action: IntegrationAction,
    },
    /// Consulta e altera preferências.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Mede recuperação, tokens, precisão e latência.
    Benchmark {
        /// Tarefas por cenário.
        #[arg(long, default_value_t = 10)]
        tasks: usize,
        /// Formato do relatório.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
        /// Grava o relatório em JSON.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Semente da seleção de tarefas, para repetibilidade.
        #[arg(long, default_value_t = 20_260_319)]
        seed: u64,
        /// Cenários separados por vírgula.
        #[arg(long, value_name = "LIST")]
        kind: Option<String>,
        /// Profundidade do acerto.
        #[arg(long, default_value_t = 5)]
        top_k: usize,
        /// Não inclui as consultas escritas à mão.
        #[arg(long)]
        no_curated: bool,
    },
    /// Inspeciona e executa a migração a partir de outras ferramentas.
    Migrate {
        /// Executa a migração.
        #[arg(long)]
        confirm: bool,
        /// Apenas mostra o plano.
        #[arg(long)]
        dry_run: bool,
        /// Escopo da integração: project, local ou user.
        #[arg(long, default_value = "project")]
        scope: String,
    },
    /// Remove a integração instalada pelo devzymer.
    Uninstall {
        /// Escopo da integração: project, local ou user.
        #[arg(long, default_value = "project")]
        scope: String,
        /// Remove também o índice local.
        #[arg(long)]
        purge: bool,
    },
    /// Executa o servidor MCP por stdio.
    Serve {
        /// Raiz do workspace.
        #[arg(long, value_name = "DIR")]
        root: Option<PathBuf>,
    },
    /// Observa o sistema de arquivos e sincroniza o índice.
    Watch,
    /// Executa um manipulador de hook do Claude Code.
    Hook {
        #[command(subcommand)]
        kind: HookKind,
    },
    /// Atualiza o devzymer para a última versão publicada.
    Update {
        /// Apenas informa se há versão nova, sem instalar.
        #[arg(long)]
        check: bool,
        /// Instala uma versão específica, por exemplo v0.1.2.
        #[arg(long, value_name = "VERSAO")]
        version: Option<String>,
        /// Atualiza mesmo que o executável pareça uma compilação local.
        #[arg(long)]
        force: bool,
    },
    /// Mostra e ajusta o roteamento do agente para o devzymer.
    Routing {
        #[command(subcommand)]
        action: RoutingAction,
    },
    /// Resolve uma consulta no índice, sem MCP.
    Resolve {
        /// Texto da consulta.
        query: String,
        /// Limite de resultados.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
}

#[derive(clap::Subcommand)]
enum IntegrationAction {
    /// Lista os agentes e o estado da integração.
    List,
    /// Instala a integração em um agente.
    Add {
        /// Agente: claude, codex ou opencode.
        agent: String,
    },
    /// Remove a integração de um agente.
    Remove {
        /// Agente: claude, codex ou opencode.
        agent: String,
    },
    /// Reaplica e verifica as integrações instaladas.
    Repair,
    /// Verifica a conectividade do servidor MCP.
    Verify,
    /// Muda o escopo de um agente.
    Scope {
        /// Agente: claude, codex ou opencode.
        agent: String,
        /// Escopo: project ou user.
        scope: String,
    },
}

#[derive(clap::ValueEnum, Clone, Copy)]
enum OutputFormat {
    Text,
    Json,
    Markdown,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Lista as preferências efetivas.
    List,
    /// Lê uma preferência.
    Get {
        /// Chave, por exemplo index.max_file_size_bytes.
        key: String,
    },
    /// Altera uma preferência.
    Set {
        /// Chave.
        key: String,
        /// Valor.
        value: String,
    },
    /// Caminho do arquivo de configuração do projeto.
    Path,
}

#[derive(Subcommand)]
enum RoutingAction {
    /// Modo, mecanismo de cada host, saúde do índice e resumo recente.
    Status,
    /// Mostra ou altera o modo: off, prefer ou guard.
    Mode {
        /// Novo modo. Sem valor, mostra o atual.
        value: Option<String>,
    },
    /// Simula a decisão do roteamento para uma chamada de ferramenta, sem registrar nada.
    Explain {
        /// Nome da ferramenta, por exemplo Grep, Glob, Read, Bash ou shell.
        #[arg(long)]
        tool: String,
        /// Entrada da ferramenta em JSON.
        #[arg(long)]
        input: String,
        /// Host: claude, codex ou opencode.
        #[arg(long)]
        host: Option<String>,
        /// Força um modo na simulação.
        #[arg(long)]
        mode: Option<String>,
    },
    /// Resumo agregado de telemetria.
    Stats {
        /// Janela em dias.
        #[arg(long, default_value_t = 7)]
        days: u64,
    },
    /// Apaga telemetria, sessões e cache de saúde do roteamento.
    Reset,
}

#[derive(Subcommand)]
enum HookKind {
    /// Comprime a saída de ferramentas grandes.
    PostToolUse,
    /// Impede a criação de arquivos de teste sem aprovação.
    GuardTestFiles,
    /// Orienta (ou, no modo guard, redireciona) a busca de codigo do agente.
    GuardSearch {
        /// Host que chamou o hook: claude, codex ou opencode.
        #[arg(long, value_name = "HOST")]
        host: Option<String>,
    },
    /// Registra o resultado de uma chamada ao devzymer, para o roteamento.
    NoteMcp {
        /// Host que chamou o hook: claude, codex ou opencode.
        #[arg(long, value_name = "HOST")]
        host: Option<String>,
    },
    /// Registra que o usuario pediu busca nativa explicitamente.
    NotePrompt {
        /// Host que chamou o hook: claude, codex ou opencode.
        #[arg(long, value_name = "HOST")]
        host: Option<String>,
    },
}

struct Ctx {
    root: PathBuf,
    config: Config,
    json: bool,
}

impl Ctx {
    /// The workspace this command operates on.
    ///
    /// The root was already validated when the command started, so it is only
    /// canonicalized here. It is never replaced by an ancestor.
    fn resolve_root(&self) -> PathBuf {
        devzymer_core::util::canonical_display(&self.root)
    }
}

fn main() {
    if let Err(error) = run() {
        let code = error
            .downcast_ref::<devzymer_core::Error>()
            .map(|inner| inner.code());
        match code {
            Some(code) => eprintln!("Error [{code}]: {error}"),
            None => eprintln!("Error: {error}"),
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let command = match cli.command {
        Some(command) => command,
        None => return home(),
    };
    let command = match command {
        Command::Hook { kind } => return cmd_hook::run(kind),
        other => other,
    };
    let cwd = devzymer_core::paths::current_dir();
    let policy = devzymer_core::workspace_root::Policy::explicit();
    let root = match devzymer_core::workspace_root::resolve(cli.root.as_deref(), &cwd, policy) {
        Ok(workspace) => workspace.root,
        Err(error) => {
            // `init` is the command that creates a workspace, so it may run
            // where no workspace exists yet. Every other command needs one.
            if !matches!(command, Command::Init { .. } | Command::Update { .. }) {
                return Err(anyhow::Error::new(error));
            }
            match cli.root.as_deref() {
                Some(path) => devzymer_core::workspace_root::canonical(path)?,
                None => devzymer_core::workspace_root::canonical(&cwd)?,
            }
        }
    };

    // Automatic discovery never lands on the user home or a filesystem root:
    // initializing there would index unrelated personal files. An explicit
    // `--root` is the way to ask for such a directory.
    if matches!(command, Command::Init { .. })
        && cli.root.is_none()
        && devzymer_core::workspace_root::is_broad(&root)
    {
        return Err(anyhow::Error::new(devzymer_core::Error::WorkspaceTooBroad(
            format!(
                "{} é a pasta do usuário ou um ancestral dela; o devzymer não inicializa                  projetos por descoberta automática nesse diretório. Entre na pasta do                  projeto ou use `devzymer init --root <pasta>`",
                root.display()
            ),
        )));
    }

    let mut config = Config::load(&root).unwrap_or_default();
    if cli.verbose {
        config.logging.level = "debug".to_string();
    }
    devzymer_core::logging::init(&config);

    let ctx = Ctx {
        root,
        config,
        json: cli.json,
    };

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        match command {
            Command::Init {
                full,
                no_integrate,
                dry_run,
                scope,
                server_only,
                agents,
                yes,
                no_agent_setup,
                repair,
            } => {
                cmd_init::run(
                    &ctx,
                    cmd_init::InitOptions {
                        full,
                        no_integrate,
                        dry_run,
                        scope,
                        server_only,
                        agents,
                        yes,
                        no_agent_setup,
                        repair,
                    },
                )
                .await
            }
            Command::Status => cmd_status::run(&ctx),
            Command::Sync { full, no_graph } => cmd_sync(&ctx, full, no_graph).await,
            Command::Doctor => cmd_doctor::run(&ctx),
            Command::Maintain { vacuum } => cmd_maintain::run(&ctx, vacuum),
            Command::Integrations { action } => cmd_integrations::run(&ctx, action),
            Command::Config { action } => cmd_config::run(&ctx, action),
            Command::Benchmark {
                tasks,
                format,
                output,
                seed,
                kind,
                top_k,
                no_curated,
            } => cmd_bench::run(&ctx, tasks, format, output, seed, kind, top_k, !no_curated),
            Command::Migrate {
                confirm,
                dry_run,
                scope,
            } => cmd_migrate::run(&ctx, confirm, dry_run, &scope),
            Command::Uninstall { scope, purge } => cmd_uninstall(&ctx, &scope, purge),
            Command::Serve { root: serve_root } => cmd_serve::run(serve_root.as_deref()).await,
            Command::Watch => cmd_watch::run(&ctx).await,
            Command::Hook { kind } => cmd_hook::run(kind),
            Command::Routing { action } => cmd_routing::run(&ctx, action),
            Command::Update {
                check,
                version,
                force,
            } => {
                cmd_update::run(
                    &ctx,
                    cmd_update::UpdateOptions {
                        check,
                        version,
                        force,
                    },
                )
                .await
            }
            Command::Resolve { query, limit } => cmd_resolve(&ctx, &query, limit),
        }
    })
}

fn home() -> Result<()> {
    use clap::CommandFactory;

    let caps = devzymer_tui::brand::detect();
    if !caps.color {
        Cli::command().print_help()?;
        return Ok(());
    }
    let version = env!("CARGO_PKG_VERSION");
    devzymer_tui::brand::print_banner(&caps, true, version)?;
    devzymer_tui::brand::heading(&caps, "Comece por aqui")?;
    let entries = [
        (
            "devzymer init",
            "indexa o projeto e conecta os seus agentes",
        ),
        ("devzymer status", "cobertura, atualidade e integração"),
        ("devzymer doctor", "diagnóstico completo"),
        ("devzymer resolve \"...\"", "busca no índice, sem MCP"),
        ("devzymer update", "atualiza o devzymer"),
        ("devzymer --help", "todos os comandos"),
    ];
    for (command, detail) in entries {
        devzymer_tui::brand::line(&caps, devzymer_tui::brand::Mark::Info, command, detail)?;
    }
    println!();
    Ok(())
}

async fn cmd_sync(ctx: &Ctx, full: bool, no_graph: bool) -> Result<()> {
    cmd_init::sync(ctx, full, no_graph).await
}

fn cmd_uninstall(ctx: &Ctx, scope: &str, purge: bool) -> Result<()> {
    cmd_migrate::uninstall(ctx, scope, purge)
}

/// Opens the index of a workspace for reading.
///
/// A command that only reads never creates an index, and the error tells the
/// user which project holds one when the requested path is inside a project.
fn open_existing_store(root: &Path) -> Result<devzymer_core::store::Store> {
    let db = paths::db_path(root);
    if !db.exists() {
        let mut message = format!(
            "indice nao encontrado em {}. Execute `devzymer init` primeiro.",
            db.display()
        );
        if let Some(project) = nearest_indexed_project(root) {
            message.push_str(&format!(
                "\nO projeto que contem esse caminho tem indice em {}.",
                project.display()
            ));
        }
        anyhow::bail!(message);
    }
    Ok(devzymer_core::store::Store::open(&db)?)
}

/// The closest ancestor of a directory that holds a devzymer index.
fn nearest_indexed_project(root: &Path) -> Option<PathBuf> {
    let mut current = root.parent();
    while let Some(dir) = current {
        if paths::db_path(dir).exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn cmd_resolve(ctx: &Ctx, query: &str, limit: usize) -> Result<()> {
    use devzymer_search::resolve::ResolveQuery;
    use devzymer_search::Resolver;

    let root = ctx.resolve_root();
    let store = open_existing_store(&root)?;
    let resolver = Resolver::new(&store, &ctx.config);
    let set = resolver.resolve(&ResolveQuery {
        query: query.to_string(),
        path: None,
        language: None,
        limit,
        candidate_limit: 0,
        diagnostics: false,
    })?;
    let results = set.results;

    if ctx.json {
        let payload: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "path": r.symbol.path,
                    "name": r.symbol.name,
                    "qualified_name": r.symbol.qualified_name,
                    "kind": r.symbol.kind.display(),
                    "start_line": r.symbol.span.start_line,
                    "end_line": r.symbol.span.end_line,
                    "confidence": r.confidence.display(),
                    "source": r.source.display(),
                    "score": r.score
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if results.is_empty() && set.files.is_empty() {
        println!("nenhum símbolo corresponde a '{query}'");
        return Ok(());
    }
    for file in &set.files {
        println!(
            "arquivo: {} | {} símbolo(s) | {}",
            file.path, file.symbols, file.reason
        );
    }
    for r in &results {
        println!(
            "{}:{}-{} | {} | {} | {}",
            r.symbol.path,
            r.symbol.span.start_line,
            r.symbol.span.end_line,
            r.symbol.qualified_name,
            r.symbol.kind,
            r.confidence.display()
        );
    }
    Ok(())
}
