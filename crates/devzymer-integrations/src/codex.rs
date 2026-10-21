//! Adaptador de instalação para o Codex.
//!
//! O Codex lê `~/.codex/config.toml` (usuário) e `<projeto>/.codex/config.toml`
//! (projeto), e registra servidores MCP em `[mcp_servers.<nome>]`. As
//! instruções do agente ficam em `AGENTS.md`.
//!
//! O Codex não usa o protocolo de hooks do Claude Code, então o instalador não
//! registra hooks: apenas a configuração MCP e as instruções.

use std::path::{Path, PathBuf};

use anyhow::Result;
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::agents::AgentKind;
use crate::claude::{backup_file, Change, InstallReport, InstallScope};

/// Caminho da configuração global do Codex.
pub fn user_config_path() -> PathBuf {
    devzymer_core::paths::user_home()
        .join(".codex")
        .join("config.toml")
}

/// Caminho da configuração do Codex no projeto.
pub fn project_config_path(root: &Path) -> PathBuf {
    root.join(".codex").join("config.toml")
}

/// Caminho das instruções do Codex no projeto.
pub fn project_rules_path(root: &Path) -> PathBuf {
    root.join("AGENTS.md")
}

/// Caminho das instruções globais do Codex.
pub fn user_rules_path() -> PathBuf {
    devzymer_core::paths::user_home()
        .join(".codex")
        .join("AGENTS.md")
}

/// O arquivo de configuração que o escopo indicado usa.
pub fn config_path(root: &Path, scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::User => user_config_path(),
        InstallScope::Project | InstallScope::Local => project_config_path(root),
    }
}

/// O arquivo de instruções que o escopo indicado usa.
pub fn rules_path(root: &Path, scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::User => user_rules_path(),
        InstallScope::Project | InstallScope::Local => project_rules_path(root),
    }
}

/// Registra o servidor MCP do devzymer na configuração do Codex.
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
    let mut document: DocumentMut = existing.parse().map_err(|e| {
        anyhow::anyhow!(
            "configuração do Codex inválida em {}: {e}",
            target.display()
        )
    })?;

    let mut args = Array::new();
    args.push("serve");
    args.push("--root");
    args.push(".");

    let servers = document
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    if let Some(table) = servers.as_table_like_mut() {
        let entry = table.entry("devzymer").or_insert(Item::Table(Table::new()));
        if let Some(entry) = entry.as_table_like_mut() {
            entry.insert("command", value(executable.display().to_string()));
            entry.insert("args", value(args.clone()));
            entry.insert("enabled", value(true));
        }
    }

    let rendered = document.to_string();
    if rendered == existing {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "servidor MCP do Codex já registrado".into(),
        });
    } else {
        backup_file(root, &target, report)?;
        devzymer_core::config::write_atomic(&target, rendered.as_bytes())?;
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "updated".into(),
            detail: "servidor MCP 'devzymer' registrado no Codex".into(),
        });
    }
    Ok(())
}

/// Remove o servidor MCP do devzymer da configuração do Codex.
pub fn uninstall(root: &Path, scope: InstallScope, report: &mut InstallReport) -> Result<()> {
    let target = config_path(root, scope);
    if !target.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&target)?;
    let mut document: DocumentMut = match existing.parse() {
        Ok(document) => document,
        Err(_) => return Ok(()),
    };
    let removed = document
        .get_mut("mcp_servers")
        .and_then(|servers| servers.as_table_like_mut())
        .map(|servers| servers.remove("devzymer").is_some())
        .unwrap_or(false);
    if !removed {
        return Ok(());
    }
    backup_file(root, &target, report)?;
    devzymer_core::config::write_atomic(&target, document.to_string().as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "servidor MCP 'devzymer' removido do Codex".into(),
    });
    Ok(())
}

/// True when the Codex configuration already points to the devzymer server.
pub fn is_installed(root: &Path, scope: InstallScope) -> bool {
    let target = config_path(root, scope);
    let Ok(text) = std::fs::read_to_string(&target) else {
        return false;
    };
    let Ok(document) = text.parse::<DocumentMut>() else {
        return false;
    };
    document
        .get("mcp_servers")
        .and_then(|servers| servers.get("devzymer"))
        .is_some()
}

/// The agent this adapter serves.
pub const AGENT: AgentKind = AgentKind::Codex;
