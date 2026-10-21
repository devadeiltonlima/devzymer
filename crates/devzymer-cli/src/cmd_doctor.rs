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

    if db.exists() {
        let store = Store::open(&db)?;
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
        checks.push(Check {
            name: "fts5",
            status: if store.fts_available() { "ok" } else { "falha" },
            detail: if store.fts_available() {
                "busca lexical disponível".to_string()
            } else {
                "a tabela symbol_fts não existe".to_string()
            },
        });
        let pending = crate::cmd_status::pending_changes(&root);
        checks.push(Check {
            name: "frescor",
            status: if pending == 0 { "ok" } else { "aviso" },
            detail: format!("{pending} arquivo(s) alterado(s) desde o último índice"),
        });
    } else {
        checks.push(Check {
            name: "banco",
            status: "aviso",
            detail: format!("índice ausente em {}", db.display()),
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
