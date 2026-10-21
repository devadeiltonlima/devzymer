//! Adaptador de instalação para o Codex.
//!
//! O Codex lê `~/.codex/config.toml` (usuário) e `<projeto>/.codex/config.toml`
//! (projeto), e registra servidores MCP em `[mcp_servers.<nome>]`. As
//! instruções do agente ficam em `AGENTS.md`.
//!
//! O Codex tem hooks de ciclo de vida (`hooks.json`) com um contrato parecido com
//! o do Claude Code. O instalador registra os hooks de roteamento, mas o Codex só
//! os executa depois que o usuário os revisa e confia neles em `/hooks`. Até lá,
//! vale apenas a orientação do `AGENTS.md`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};
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

const HOOK_NAMES: [&str; 3] = ["guard-search", "note-mcp", "note-prompt"];

/// O arquivo de hooks que o escopo indicado usa.
pub fn hooks_path(root: &Path, scope: InstallScope) -> PathBuf {
    match scope {
        InstallScope::User => devzymer_core::paths::user_home()
            .join(".codex")
            .join("hooks.json"),
        InstallScope::Project | InstallScope::Local => root.join(".codex").join("hooks.json"),
    }
}

fn shell_word(text: &str) -> String {
    if text.contains(char::is_whitespace) {
        format!("\"{text}\"")
    } else {
        text.to_string()
    }
}

fn hook_command(executable: &Path, hook: &str) -> String {
    format!(
        "{} hook {hook} --host codex",
        shell_word(&executable.display().to_string())
    )
}

fn hook_handler(executable: &Path, hook: &str, timeout: u32) -> Value {
    let command = hook_command(executable, hook);
    json!({
        "type": "command",
        "command": command,
        "commandWindows": command,
        "timeout": timeout
    })
}

fn is_ours(group: &Value) -> bool {
    HOOK_NAMES
        .iter()
        .any(|hook| crate::claude::group_runs(group, hook))
}

fn upsert_group(hooks: &mut Value, event: &str, group: Value, hook: &str) {
    let Some(map) = hooks.as_object_mut() else {
        return;
    };
    let entry = map
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !entry.is_array() {
        *entry = Value::Array(Vec::new());
    }
    let Some(list) = entry.as_array_mut() else {
        return;
    };
    let position = list
        .iter()
        .position(|existing| crate::claude::group_runs(existing, hook));
    match position {
        Some(keep) => {
            list[keep] = group;
            let mut index = 0usize;
            list.retain(|existing| {
                let current = index;
                index += 1;
                current == keep || !crate::claude::group_runs(existing, hook)
            });
        }
        None => list.push(group),
    }
}

/// Registra os hooks de roteamento do devzymer no Codex.
///
/// O Codex só executa hooks de projeto depois de o usuário revisá-los em
/// `/hooks`, então a instalação grava o arquivo e avisa disso.
pub fn install_hooks(
    root: &Path,
    scope: InstallScope,
    executable: &Path,
    report: &mut InstallReport,
) -> Result<()> {
    let target = hooks_path(root, scope);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let guard = json!({
        "matcher": "Bash",
        "hooks": [hook_handler(executable, "guard-search", 10)]
    });
    let note_mcp = json!({
        "matcher": "mcp__devzymer__.*",
        "hooks": [hook_handler(executable, "note-mcp", 10)]
    });
    let note_prompt = json!({
        "hooks": [hook_handler(executable, "note-prompt", 10)]
    });
    let changed = crate::claude::update_json_file(&target, |document| {
        let Some(root_map) = document.as_object_mut() else {
            return false;
        };
        let hooks = root_map.entry("hooks").or_insert_with(|| json!({}));
        upsert_group(hooks, "PreToolUse", guard.clone(), "guard-search");
        upsert_group(hooks, "PostToolUse", note_mcp.clone(), "note-mcp");
        upsert_group(
            hooks,
            "UserPromptSubmit",
            note_prompt.clone(),
            "note-prompt",
        );
        true
    })?;
    if !changed {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "os hooks de roteamento do Codex já estão instalados".into(),
        });
        return Ok(());
    }
    backup_file(root, &target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "hooks de roteamento gravados; o Codex só os executa depois de você revisá-los em /hooks"
            .into(),
    });
    Ok(())
}

/// Remove os hooks de roteamento do devzymer do Codex.
pub fn uninstall_hooks(root: &Path, scope: InstallScope, report: &mut InstallReport) -> Result<()> {
    let target = hooks_path(root, scope);
    if !target.exists() {
        return Ok(());
    }
    let changed = crate::claude::update_json_file(&target, |document| {
        let Some(hooks) = document.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
            return false;
        };
        let mut touched = false;
        for list in hooks.values_mut().filter_map(|l| l.as_array_mut()) {
            let before = list.len();
            list.retain(|group| !is_ours(group));
            touched |= before != list.len();
        }
        touched
    })?;
    if changed {
        backup_file(root, &target, report)?;
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "updated".into(),
            detail: "hooks de roteamento do devzymer removidos do Codex".into(),
        });
    }
    Ok(())
}

/// True when the routing hooks are written in the hooks file of the scope.
pub fn hooks_installed(root: &Path, scope: InstallScope) -> bool {
    let Ok(text) = std::fs::read_to_string(hooks_path(root, scope)) else {
        return false;
    };
    let Ok(document) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    document
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(|l| l.as_array())
        .map(|list| {
            list.iter()
                .any(|group| crate::claude::group_runs(group, "guard-search"))
        })
        .unwrap_or(false)
}

/// The agent this adapter serves.
pub const AGENT: AgentKind = AgentKind::Codex;

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-codex-hooks-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_hooks_are_installed_once_and_keep_foreign_hooks() {
        let root = project("foreign");
        let path = hooks_path(&root, InstallScope::Project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"audit.sh"}]}]}}"#,
        )
        .unwrap();
        let exe = Path::new("C:/tools/devzymer.exe");
        let mut report = InstallReport::default();
        install_hooks(&root, InstallScope::Project, exe, &mut report).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        let mut again = InstallReport::default();
        install_hooks(&root, InstallScope::Project, exe, &mut again).unwrap();
        assert_eq!(first, std::fs::read_to_string(&path).unwrap());
        assert_eq!(again.changes[0].action, "unchanged");

        let document: Value = serde_json::from_str(&first).unwrap();
        let pre = document["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0]["hooks"][0]["command"], "audit.sh");
        assert_eq!(
            pre[1]["hooks"][0]["command"],
            "C:/tools/devzymer.exe hook guard-search --host codex"
        );
        assert!(document["hooks"]["PostToolUse"].is_array());
        assert!(document["hooks"]["UserPromptSubmit"].is_array());
        assert!(hooks_installed(&root, InstallScope::Project));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_moved_binary_replaces_the_group_instead_of_duplicating_it() {
        let root = project("moved");
        let mut report = InstallReport::default();
        install_hooks(
            &root,
            InstallScope::Project,
            Path::new("/old/devzymer"),
            &mut report,
        )
        .unwrap();
        install_hooks(
            &root,
            InstallScope::Project,
            Path::new("/new/devzymer"),
            &mut report,
        )
        .unwrap();
        let text = std::fs::read_to_string(hooks_path(&root, InstallScope::Project)).unwrap();
        assert!(text.contains("/new/devzymer"));
        assert!(!text.contains("/old/devzymer"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn uninstall_removes_only_the_devzymer_hooks() {
        let root = project("uninstall");
        let path = hooks_path(&root, InstallScope::Project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"audit.sh"}]}]}}"#,
        )
        .unwrap();
        let mut report = InstallReport::default();
        install_hooks(
            &root,
            InstallScope::Project,
            Path::new("/x/devzymer"),
            &mut report,
        )
        .unwrap();
        uninstall_hooks(&root, InstallScope::Project, &mut report).unwrap();
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pre = document["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["hooks"][0]["command"], "audit.sh");
        assert!(!hooks_installed(&root, InstallScope::Project));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_path_with_spaces_is_quoted() {
        assert_eq!(
            hook_command(Path::new("C:/Program Files/dz/devzymer.exe"), "note-mcp"),
            "\"C:/Program Files/dz/devzymer.exe\" hook note-mcp --host codex"
        );
    }
}
