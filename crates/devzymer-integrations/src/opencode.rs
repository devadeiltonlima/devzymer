//! Adaptador de instalação para o OpenCode.
//!
//! O OpenCode lê `opencode.json` no projeto e `opencode.json` ou
//! `opencode.jsonc` na configuração global. Os servidores MCP ficam em
//! `mcp.servers.<nome>`, e as instruções do agente em `AGENTS.md`.
//!
//! O arquivo global pode ser JSON com comentários, então a alteração é feita
//! sobre o texto original para preservar o que o usuário escreveu.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::agents::AgentKind;
use crate::claude::{backup_file, Change, InstallReport, InstallScope};
use crate::jsonc;

/// Caminho da configuração global do OpenCode.
pub fn user_config_path() -> PathBuf {
    let dir = devzymer_core::paths::user_config_dir()
        .parent()
        .map(|p| p.join("opencode"))
        .unwrap_or_else(|| {
            devzymer_core::paths::user_home()
                .join(".config")
                .join("opencode")
        });
    dir.join("opencode.json")
}

/// Caminho da configuração global do OpenCode com comentários.
pub fn user_config_jsonc_path() -> PathBuf {
    let dir = devzymer_core::paths::user_config_dir()
        .parent()
        .map(|p| p.join("opencode"))
        .unwrap_or_else(|| {
            devzymer_core::paths::user_home()
                .join(".config")
                .join("opencode")
        });
    dir.join("opencode.jsonc")
}

/// Caminho da configuração do OpenCode no projeto.
///
/// O OpenCode aceita `opencode.json` e `opencode.jsonc`. Quando o usuário
/// escolheu a versão com comentários, ela é preservada.
pub fn project_config_path(root: &Path) -> PathBuf {
    let jsonc = root.join("opencode.jsonc");
    if jsonc.exists() {
        return jsonc;
    }
    root.join("opencode.json")
}

/// Caminho das instruções do OpenCode no projeto.
pub fn project_rules_path(root: &Path) -> PathBuf {
    root.join("AGENTS.md")
}

/// O arquivo de configuração que o escopo indicado usa.
pub fn config_path(root: &Path, scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::User => {
            if user_config_jsonc_path().exists() {
                user_config_jsonc_path()
            } else {
                user_config_path()
            }
        }
        InstallScope::Project | InstallScope::Local => project_config_path(root),
    }
}

/// O arquivo de instruções que o escopo indicado usa.
pub fn rules_path(root: &Path, scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::User => devzymer_core::paths::user_home()
            .join(".config")
            .join("opencode")
            .join("AGENTS.md"),
        InstallScope::Project | InstallScope::Local => project_rules_path(root),
    }
}

/// O valor JSON do servidor MCP local do devzymer.
fn server_entry(executable: &Path) -> String {
    serde_json::json!({
        "type": "local",
        "command": [
            executable.display().to_string(),
            "serve",
            "--root",
            "."
        ],
        "enabled": true
    })
    .to_string()
}

/// Registra o servidor MCP do devzymer na configuração do OpenCode.
pub fn install(
    root: &Path,
    scope: InstallScope,
    executable: &Path,
    report: &mut InstallReport,
) -> Result<()> {
    let target = config_path(root, scope);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&target).unwrap_or_default();
    let entry = server_entry(executable);

    let updated = if existing.trim().is_empty() {
        format!("{{\n  \"mcp\": {{\n    \"servers\": {{\n      \"devzymer\": {entry}\n    }}\n  }}\n}}\n")
    } else {
        let text = jsonc::insert_member(&existing, &["mcp", "servers"], "devzymer", &entry)
            .map_err(|e| {
                anyhow::anyhow!(
                    "configuração do OpenCode inválida em {}: {e}",
                    target.display()
                )
            })?;
        // A entrada precisa ser inserida dentro de um objeto que existe.
        if text == existing && !existing.contains("devzymer") {
            anyhow::bail!(
                "a configuração do OpenCode em {} não tem a seção mcp.servers",
                target.display()
            );
        }
        text
    };

    if updated == existing {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "servidor MCP do OpenCode já registrado".into(),
        });
        return Ok(());
    }
    backup_file(root, &target, report)?;
    devzymer_core::config::write_atomic(&target, updated.as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "servidor MCP 'devzymer' registrado no OpenCode".into(),
    });
    Ok(())
}

/// Remove o servidor MCP do devzymer da configuração do OpenCode.
pub fn uninstall(root: &Path, scope: InstallScope, report: &mut InstallReport) -> Result<()> {
    let target = config_path(root, scope);
    if !target.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&target)?;
    let updated = match jsonc::replace(&existing, &["mcp", "servers", "devzymer"], "null") {
        Ok(text) => text,
        Err(_) => return Ok(()),
    };
    backup_file(root, &target, report)?;
    devzymer_core::config::write_atomic(&target, updated.as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "servidor MCP 'devzymer' removido do OpenCode".into(),
    });
    Ok(())
}

/// True when the OpenCode configuration already points to the devzymer server.
pub fn is_installed(root: &Path, scope: InstallScope) -> bool {
    let target = config_path(root, scope);
    let Ok(text) = std::fs::read_to_string(&target) else {
        return false;
    };
    jsonc::find(&text, &["mcp", "servers", "devzymer"])
        .map(|node| node.is_some())
        .unwrap_or(false)
}

/// The agent this adapter serves.
pub const AGENT: AgentKind = AgentKind::OpenCode;
