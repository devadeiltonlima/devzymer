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
        "codemode": false
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

const PLUGIN_ID: &str = "devzymer-routing";

const PLUGIN_TEMPLATE: &str = r####"import { spawnSync } from "node:child_process"

const BIN = __BIN__
const WATCHED = new Set(["grep", "glob", "read", "bash", "shell", "powershell"])

function run(kind, payload, cwd) {
  const result = spawnSync(BIN, ["hook", kind, "--host", "opencode"], {
    input: JSON.stringify(payload),
    encoding: "utf8",
    timeout: 4000,
    windowsHide: true,
    cwd,
  })
  if (result.error || result.status !== 0 || !result.stdout) return undefined
  try {
    return JSON.parse(result.stdout.trim())
  } catch {
    return undefined
  }
}

export default {
  id: "devzymer-routing",
  async setup(ctx) {
    const major = Number.parseInt(String(ctx?.app?.version ?? "0").split(".")[0], 10)
    if (!(major >= 2) || typeof ctx?.tool?.hook !== "function") return
    const cwd = ctx.location?.directory
    const advice = new Map()
    await ctx.tool.hook("execute.before", (event) => {
      if (!WATCHED.has(event.tool)) return
      const verdict = run(
        "guard-search",
        { tool_name: event.tool, tool_input: event.input, session_id: event.sessionID, cwd },
        cwd,
      )
      if (!verdict) return
      if (verdict.action === "deny") throw new Error(verdict.message)
      if (verdict.action === "advise") advice.set(event.id, verdict.message)
    })
    await ctx.tool.hook("execute.after", (event) => {
      if (event.tool.startsWith("devzymer_")) {
        run(
          "note-mcp",
          {
            tool_name: event.tool,
            tool_response: { isError: event.status !== "completed", content: event.result?.output },
            session_id: event.sessionID,
            cwd,
          },
          cwd,
        )
        return
      }
      const message = advice.get(event.id)
      if (!message) return
      advice.delete(event.id)
      if (event.result && typeof event.result.output === "string") {
        event.result.output += "\n\n" + message
      }
    })
  },
}
"####;

/// O arquivo do plugin de roteamento que o escopo indicado usa.
pub fn plugin_path(root: &Path, scope: InstallScope) -> PathBuf {
    let base = match scope {
        InstallScope::User => user_config_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join(".opencode")),
        InstallScope::Project | InstallScope::Local => root.join(".opencode"),
    };
    base.join("plugins").join(PLUGIN_ID).join("index.ts")
}

fn plugin_source(executable: &Path) -> String {
    let bin = serde_json::to_string(&executable.display().to_string())
        .unwrap_or_else(|_| "\"devzymer\"".to_string());
    PLUGIN_TEMPLATE.replace("__BIN__", &bin)
}

/// Major version of the installed OpenCode, when it can be determined.
pub fn installed_major() -> Option<u32> {
    let output = std::process::Command::new("opencode")
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.split_whitespace().find_map(|word| {
        word.trim_start_matches('v')
            .split('.')
            .next()?
            .parse::<u32>()
            .ok()
    })?;
    Some(version)
}

/// Instala o plugin de roteamento do OpenCode (API de plugins da versão 2).
///
/// Um arquivo que não é do devzymer nunca é sobrescrito. Quando a versão
/// instalada é a 1, a API é outra e o plugin não é instalado: o roteamento
/// fica só na orientação do `AGENTS.md`.
pub fn install_plugin(
    root: &Path,
    scope: InstallScope,
    executable: &Path,
    major: Option<u32>,
    report: &mut InstallReport,
) -> Result<()> {
    let target = plugin_path(root, scope);
    if major.is_some_and(|major| major < 2) {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "skipped".into(),
            detail: "OpenCode 1 usa outra API de plugins; roteamento apenas por orientação".into(),
        });
        return Ok(());
    }
    let existing = std::fs::read_to_string(&target).ok();
    if let Some(text) = &existing {
        if !text.contains(PLUGIN_ID) {
            report.changes.push(Change {
                file: target.display().to_string(),
                action: "skipped".into(),
                detail: "já existe um plugin com esse nome que não é do devzymer; não foi alterado"
                    .into(),
            });
            return Ok(());
        }
    }
    let source = plugin_source(executable);
    if existing.as_deref() == Some(source.as_str()) {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "plugin de roteamento do OpenCode já instalado".into(),
        });
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if existing.is_some() {
        backup_file(root, &target, report)?;
    }
    devzymer_core::config::write_atomic(&target, source.as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "plugin de roteamento do OpenCode instalado".into(),
    });
    Ok(())
}

/// Remove o plugin de roteamento, se for do devzymer.
pub fn uninstall_plugin(
    root: &Path,
    scope: InstallScope,
    report: &mut InstallReport,
) -> Result<()> {
    let target = plugin_path(root, scope);
    let Ok(text) = std::fs::read_to_string(&target) else {
        return Ok(());
    };
    if !text.contains(PLUGIN_ID) {
        return Ok(());
    }
    backup_file(root, &target, report)?;
    std::fs::remove_file(&target)?;
    if let Some(parent) = target.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "removed".into(),
        detail: "plugin de roteamento do devzymer removido do OpenCode".into(),
    });
    Ok(())
}

/// True when the routing plugin of devzymer is in place for the scope.
pub fn plugin_installed(root: &Path, scope: InstallScope) -> bool {
    std::fs::read_to_string(plugin_path(root, scope))
        .map(|text| text.contains(PLUGIN_ID))
        .unwrap_or(false)
}

/// The agent this adapter serves.
pub const AGENT: AgentKind = AgentKind::OpenCode;

#[cfg(test)]
mod plugin_tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-oc-plugin-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_plugin_embeds_the_binary_path_and_is_idempotent() {
        let root = project("install");
        let exe = Path::new("C:\\tools\\devzymer.exe");
        let mut report = InstallReport::default();
        install_plugin(&root, InstallScope::Project, exe, Some(2), &mut report).unwrap();
        let text = std::fs::read_to_string(plugin_path(&root, InstallScope::Project)).unwrap();
        assert!(text.contains("C:\\\\tools\\\\devzymer.exe"));
        assert!(text.contains("execute.before"));
        let mut again = InstallReport::default();
        install_plugin(&root, InstallScope::Project, exe, Some(2), &mut again).unwrap();
        assert_eq!(again.changes[0].action, "unchanged");
        assert!(plugin_installed(&root, InstallScope::Project));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_foreign_plugin_with_the_same_path_is_never_overwritten() {
        let root = project("foreign");
        let path = plugin_path(&root, InstallScope::Project);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "export default { id: \"mine\" }").unwrap();
        let mut report = InstallReport::default();
        install_plugin(
            &root,
            InstallScope::Project,
            Path::new("/x/devzymer"),
            Some(2),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.changes[0].action, "skipped");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "export default { id: \"mine\" }"
        );
        uninstall_plugin(&root, InstallScope::Project, &mut report).unwrap();
        assert!(path.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_server_is_registered_with_direct_tools_and_without_the_v1_enabled_flag() {
        let entry: serde_json::Value =
            serde_json::from_str(&server_entry(Path::new("/x/devzymer"))).unwrap();
        assert_eq!(entry["codemode"], false);
        assert_eq!(entry["type"], "local");
        assert!(entry.get("enabled").is_none());
    }

    #[test]
    fn opencode_one_gets_guidance_only() {
        let root = project("v1");
        let mut report = InstallReport::default();
        install_plugin(
            &root,
            InstallScope::Project,
            Path::new("/x/devzymer"),
            Some(1),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.changes[0].action, "skipped");
        assert!(!plugin_path(&root, InstallScope::Project).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn uninstall_removes_the_devzymer_plugin() {
        let root = project("remove");
        let mut report = InstallReport::default();
        install_plugin(
            &root,
            InstallScope::Project,
            Path::new("/x/devzymer"),
            None,
            &mut report,
        )
        .unwrap();
        uninstall_plugin(&root, InstallScope::Project, &mut report).unwrap();
        assert!(!plugin_installed(&root, InstallScope::Project));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
