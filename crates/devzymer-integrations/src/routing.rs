//! What enforces the routing policy on each host, read from the files that
//! are actually on disk. Nothing here claims more than the files prove.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::agents::AgentKind;
use crate::claude::{group_runs, InstallScope, RULES_END, RULES_START};

#[derive(Debug, Clone, Serialize)]
pub struct HostRouting {
    pub agent: AgentKind,
    pub mechanism: &'static str,
    pub mechanism_installed: bool,
    pub guidance_installed: bool,
    pub enforcement: &'static str,
    pub note: String,
}

fn has_rules(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains(RULES_START) && text.contains(RULES_END))
        .unwrap_or(false)
}

fn claude_hook_files(root: &Path) -> Vec<PathBuf> {
    let home = devzymer_core::paths::user_home();
    vec![
        root.join(".claude").join("settings.json"),
        root.join(".claude").join("settings.local.json"),
        home.join(".claude").join("settings.json"),
    ]
}

fn claude_guard_installed(root: &Path) -> bool {
    claude_hook_files(root).iter().any(|path| {
        crate::claude::read_json(path)
            .ok()
            .and_then(|document| {
                document
                    .get("hooks")
                    .and_then(|h| h.get("PreToolUse"))
                    .and_then(Value::as_array)
                    .map(|list| list.iter().any(|g| group_runs(g, "guard-search")))
            })
            .unwrap_or(false)
    })
}

pub fn inspect(root: &Path, opencode_major: Option<u32>) -> Vec<HostRouting> {
    let home = devzymer_core::paths::user_home();
    let mut hosts = Vec::new();

    let hooks = claude_guard_installed(root);
    let guidance =
        has_rules(&root.join("CLAUDE.md")) || has_rules(&home.join(".claude").join("CLAUDE.md"));
    hosts.push(HostRouting {
        agent: AgentKind::Claude,
        mechanism: "hooks",
        mechanism_installed: hooks,
        guidance_installed: guidance,
        enforcement: if hooks {
            "guard-capable"
        } else {
            "guidance-only"
        },
        note: if hooks {
            "PreToolUse bloqueia busca redundante apenas no modo guard".into()
        } else {
            "hooks ausentes: rode `devzymer init`".into()
        },
    });

    let hooks = [InstallScope::Project, InstallScope::User]
        .iter()
        .any(|scope| crate::codex::hooks_installed(root, *scope));
    let guidance = has_rules(&crate::codex::project_rules_path(root))
        || has_rules(&crate::codex::user_rules_path());
    hosts.push(HostRouting {
        agent: AgentKind::Codex,
        mechanism: "hooks",
        mechanism_installed: hooks,
        guidance_installed: guidance,
        enforcement: if hooks {
            "requires-review"
        } else {
            "guidance-only"
        },
        note: if hooks {
            "hooks gravados; o Codex só os executa depois da revisão em /hooks".into()
        } else {
            "sem hooks: orientação do AGENTS.md, sem garantia técnica".into()
        },
    });

    let plugin = [InstallScope::Project, InstallScope::User]
        .iter()
        .any(|scope| crate::opencode::plugin_installed(root, *scope));
    let guidance = has_rules(&crate::opencode::project_rules_path(root))
        || has_rules(&crate::opencode::rules_path(root, InstallScope::User));
    let supported = opencode_major.is_none_or(|major| major >= 2);
    hosts.push(HostRouting {
        agent: AgentKind::OpenCode,
        mechanism: "plugin",
        mechanism_installed: plugin,
        guidance_installed: guidance,
        enforcement: if plugin && supported {
            "guard-capable"
        } else {
            "guidance-only"
        },
        note: match (plugin, supported) {
            (true, true) => "plugin com execute.before (API de plugins do OpenCode 2)".into(),
            (_, false) => "OpenCode 1: sem plugin; apenas orientação".into(),
            (false, true) => "plugin ausente: rode `devzymer init`".into(),
        },
    });
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_project_reports_guidance_only_everywhere_it_has_no_mechanism() {
        let root = std::env::temp_dir().join(format!(
            "devzymer-routing-inspect-{}",
            devzymer_core::util::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let hosts = inspect(&root, Some(2));
        let opencode = hosts
            .iter()
            .find(|h| h.agent == AgentKind::OpenCode)
            .unwrap();
        assert!(!opencode.mechanism_installed);
        assert_eq!(opencode.enforcement, "guidance-only");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn opencode_one_is_never_guard_capable() {
        let root = std::env::temp_dir().join(format!(
            "devzymer-routing-inspect1-{}",
            devzymer_core::util::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut report = crate::InstallReport::default();
        crate::opencode::install_plugin(
            &root,
            InstallScope::Project,
            Path::new("/x/devzymer"),
            None,
            &mut report,
        )
        .unwrap();
        let hosts = inspect(&root, Some(1));
        let opencode = hosts
            .iter()
            .find(|h| h.agent == AgentKind::OpenCode)
            .unwrap();
        assert_eq!(opencode.enforcement, "guidance-only");
        let current = inspect(&root, Some(2));
        assert_eq!(
            current
                .iter()
                .find(|h| h.agent == AgentKind::OpenCode)
                .unwrap()
                .enforcement,
            "guard-capable"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
