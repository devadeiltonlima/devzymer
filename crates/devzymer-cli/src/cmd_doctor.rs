use anyhow::Result;
use devzymer_core::paths;
use devzymer_core::store::Store;
use devzymer_core::workspace::Workspace;

use crate::Ctx;

#[derive(serde::Serialize)]
struct Check {
    name: &'static str,
    status: &'static str,
    detail: String,
}

pub fn run(ctx: &Ctx) -> Result<()> {
    let root = ctx.resolve_root();
    let workspace = Workspace::detect(&root);
    let db = paths::db_path(&root);
    let mut checks: Vec<Check> = Vec::new();

    checks.push(Check {
        name: "workspace",
        status: if root.exists() { "ok" } else { "falha" },
        detail: root.display().to_string(),
    });

    checks.push(Check {
        name: "git",
        status: if workspace.git.is_some() {
            "ok"
        } else {
            "aviso"
        },
        detail: match &workspace.git {
            Some(git) => format!("raiz {}, worktree={}", git.root, git.is_worktree),
            None => "repositório git não detectado".to_string(),
        },
    });

    let opened = if db.exists() {
        match Store::open(&db) {
            Ok(store) => Some(store),
            Err(error) => {
                checks.push(Check {
                    name: "banco",
                    status: "falha",
                    detail: format!(
                        "não foi possível abrir o índice: {error}. Nada foi alterado;                          `devzymer init --repair` guarda-o ao lado e reconstrói"
                    ),
                });
                None
            }
        }
    } else {
        None
    };
    if let Some(store) = opened {
        let integrity = store.integrity_check()?;
        let stats = store.stats()?;
        checks.push(Check {
            name: "banco",
            status: if integrity == "ok" { "ok" } else { "falha" },
            detail: format!(
                "{} | {} arquivos, {} símbolos, {} relações | revisão {}",
                integrity, stats.files, stats.symbols, stats.relations, stats.revision
            ),
        });
        let fts = store.fts_integrity_check();
        checks.push(Check {
            name: "fts5",
            status: if fts.is_ok() { "ok" } else { "falha" },
            detail: match &fts {
                Ok(()) => "busca lexical disponível e consistente".to_string(),
                Err(error) => format!("índice lexical com problema: {error}"),
            },
        });
        let parser_stale = store.parser_version_changed();
        checks.push(Check {
            name: "parser",
            status: if parser_stale { "aviso" } else { "ok" },
            detail: if parser_stale {
                format!(
                    "índice feito com o parser {} (atual: {}); `devzymer sync` reconstrói",
                    store
                        .indexed_parser_version()
                        .unwrap_or_else(|| "desconhecido".to_string()),
                    store.parser_version()
                )
            } else {
                format!("parser {}", store.parser_version())
            },
        });
        let pending = crate::cmd_status::pending_changes(&root);
        checks.push(Check {
            name: "frescor",
            status: if pending == 0 { "ok" } else { "aviso" },
            detail: format!("{pending} arquivo(s) alterado(s) desde o último índice"),
        });
    } else if !db.exists() {
        checks.push(Check {
            name: "banco",
            status: "aviso",
            detail: format!("índice ausente em {}", db.display()),
        });
    }

    if let Some(state) = crate::init_state::read(&root) {
        let complete = state.stage == crate::init_state::Stage::Complete;
        checks.push(Check {
            name: "inicialização",
            status: if complete { "ok" } else { "aviso" },
            detail: if complete {
                state.stage.describe().to_string()
            } else {
                format!(
                    "{}; rode `devzymer init --repair` para concluir",
                    state.stage.describe()
                )
            },
        });
    }

    checks.push(Check {
        name: "claude-code",
        status: if workspace.claude.cli_installed {
            "ok"
        } else {
            "aviso"
        },
        detail: if workspace.claude.cli_installed {
            "CLI presente no PATH".to_string()
        } else {
            "CLI do Claude Code não encontrado no PATH".to_string()
        },
    });

    let mcp_file = workspace.claude.project_mcp.clone();
    checks.push(Check {
        name: "mcp",
        status: if mcp_file.as_ref().map(|p| p.exists()).unwrap_or(false) {
            "ok"
        } else {
            "aviso"
        },
        detail: match &mcp_file {
            Some(path) if path.exists() => path.display().to_string(),
            _ => "nenhum .mcp.json no projeto".to_string(),
        },
    });

    for detection in devzymer_integrations::detect(&root) {
        let status = match detection.status {
            devzymer_integrations::AgentStatus::Connected => "ok",
            devzymer_integrations::AgentStatus::Configured => "ok",
            devzymer_integrations::AgentStatus::Detected => "aviso",
            devzymer_integrations::AgentStatus::NotInstalled => "aviso",
        };
        let detail = match &detection.registered_at {
            Some(path) => format!("{} em {}", detection.status.display(), path.display()),
            None => detection.status.display().to_string(),
        };
        checks.push(Check {
            name: match detection.agent {
                devzymer_integrations::AgentKind::Claude => "agente:claude",
                devzymer_integrations::AgentKind::Codex => "agente:codex",
                devzymer_integrations::AgentKind::OpenCode => "agente:opencode",
            },
            status,
            detail,
        });
    }

    for (file, name) in [
        ("CLAUDE.md", "instruções:claude"),
        ("AGENTS.md", "instruções:agents"),
    ] {
        let path = root.join(file);
        let present = std::fs::read_to_string(&path).is_ok_and(|text| {
            text.contains(devzymer_integrations::claude::RULES_START)
                && text.contains(devzymer_integrations::claude::RULES_END)
        });
        checks.push(Check {
            name,
            status: if present { "ok" } else { "aviso" },
            detail: if present {
                format!("bloco do devzymer em {file}")
            } else {
                format!("sem bloco do devzymer em {file}; `devzymer init` o acrescenta")
            },
        });
    }
    for (dir, name) in [
        (
            devzymer_integrations::setup::CLAUDE_SKILL_DIR,
            "skill:claude",
        ),
        (
            devzymer_integrations::setup::AGENTS_SKILL_DIR,
            "skill:agents",
        ),
    ] {
        let inspection = devzymer_integrations::skill::inspect(&root.join(dir));
        let (status, detail) = if !inspection.installed {
            (
                "aviso",
                format!("skill ausente em {dir}; `devzymer init` a instala"),
            )
        } else if !inspection.frontmatter_ok {
            ("aviso", format!("{dir}: {}", inspection.detail))
        } else if inspection.is_current() {
            (
                "ok",
                format!(
                    "versão {} em {dir} (carregamento pelo agente não é verificado aqui)",
                    inspection.version.as_deref().unwrap_or("?")
                ),
            )
        } else {
            (
                "aviso",
                format!(
                    "{dir} difere do modelo atual (versão instalada {}); `devzymer init`                      atualiza o que é do devzymer e preserva o que você alterou",
                    inspection.version.as_deref().unwrap_or("desconhecida")
                ),
            )
        };
        checks.push(Check {
            name,
            status,
            detail,
        });
    }

    checks.push(Check {
        name: "codegraph",
        status: if workspace.codegraph.present {
            "aviso"
        } else {
            "ok"
        },
        detail: if workspace.codegraph.present {
            format!(
                "detectado: {}",
                workspace.codegraph.mcp_entries.to_vec().join(", ")
            )
        } else {
            "nenhuma integração CodeGraph encontrada".to_string()
        },
    });

    let key_env = ctx.config.decisions.api_key_env.clone();
    let has_key = std::env::var(&key_env)
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let decisions_status = if !ctx.config.decisions.enabled || has_key {
        "ok"
    } else {
        "aviso"
    };
    checks.push(Check {
        name: "decisões",
        status: decisions_status,
        detail: if !ctx.config.decisions.enabled {
            "desabilitadas; o roteamento usa apenas regras locais".to_string()
        } else if has_key {
            format!("provedor externo habilitado com {key_env}")
        } else {
            format!("provedor externo habilitado, mas {key_env} não está definida")
        },
    });

    let failures = checks.iter().filter(|c| c.status == "falha").count();

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        for check in &checks {
            println!("[{}] {}: {}", check.status, check.name, check.detail);
        }
        println!("\n{} verificação(ões), {} falha(s)", checks.len(), failures);
    }

    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}
