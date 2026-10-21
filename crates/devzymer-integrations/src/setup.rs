//! Os arquivos do agente dentro do projeto: instruções e skill.
//!
//! Tudo aqui é local ao projeto. Nada em `~/.claude`, `~/.codex` ou na
//! configuração global do OpenCode é lido ou escrito. O registro do servidor
//! MCP e os hooks continuam nos instaladores de cada host.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agents::{detect_one, AgentKind};
use crate::claude::{backup_file, InstallReport, RULES_END, RULES_START};
use crate::instructions::{self, BlockAction};
use crate::skill::{self, SkillAction};

/// Onde a skill fica para cada família de hosts.
pub const CLAUDE_SKILL_DIR: &str = ".claude/skills/devzymer";
pub const AGENTS_SKILL_DIR: &str = ".agents/skills/devzymer";

#[derive(Debug, Clone, Serialize)]
pub struct SetupItem {
    /// `instructions` ou `skill`.
    pub kind: &'static str,
    /// Caminho relativo à raiz do projeto, com `/`.
    pub path: String,
    pub action: String,
    pub detail: String,
}

/// O estado de cada host: o registro do servidor e os arquivos são coisas
/// separadas, e o carregamento real só se prova numa sessão do agente.
#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub agent: AgentKind,
    pub mcp_registered: bool,
    pub instructions_present: bool,
    pub skill_current: bool,
    pub skill_version: Option<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SetupReport {
    pub applied: bool,
    pub items: Vec<SetupItem>,
    pub warnings: Vec<String>,
    pub backups: Vec<String>,
    pub hosts: Vec<HostStatus>,
}

impl SetupReport {
    /// True when at least one file was (or would be) written.
    pub fn changed(&self) -> bool {
        self.items.iter().any(|item| {
            matches!(
                item.action.as_str(),
                "created" | "appended" | "updated" | "partial_updated"
            )
        })
    }

    /// True when something prevented a file from being prepared.
    pub fn has_blockers(&self) -> bool {
        self.items.iter().any(|item| item.action == "skipped")
    }
}

/// Canonical form used to compare paths, without the Windows `\\?\` prefix.
fn canonical(path: &Path) -> Option<PathBuf> {
    path.canonicalize()
        .ok()
        .map(|resolved| devzymer_core::util::canonical_display(&resolved))
}

/// True when `path`, after resolving every existing symlink, stays inside `root`.
fn inside_root(root: &Path, path: &Path) -> bool {
    let Some(root) = canonical(root) else {
        return false;
    };
    let mut probe = Some(path);
    while let Some(candidate) = probe {
        if candidate.symlink_metadata().is_ok() {
            return canonical(candidate).is_some_and(|resolved| resolved.starts_with(&root));
        }
        probe = candidate.parent();
    }
    false
}

fn block_present(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.contains(RULES_START) && text.contains(RULES_END))
        .unwrap_or(false)
}

fn skipped(kind: &'static str, path: String, detail: impl Into<String>) -> SetupItem {
    SetupItem {
        kind,
        path,
        action: "skipped".to_string(),
        detail: detail.into(),
    }
}

fn upsert_instructions(
    root: &Path,
    name: &str,
    block: &str,
    apply: bool,
    report: &mut SetupReport,
) {
    let path = root.join(name);
    let display = name.to_string();

    let mut target = path.clone();
    if path
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        match canonical(&path) {
            Some(real) if inside_root(root, &real) => target = real,
            Some(real) => {
                report.items.push(skipped(
                    "instructions",
                    display,
                    format!(
                        "é um link para {}, fora do projeto; nada foi alterado",
                        real.display()
                    ),
                ));
                return;
            }
            None => {
                report.items.push(skipped(
                    "instructions",
                    display,
                    "é um link quebrado; nada foi alterado",
                ));
                return;
            }
        }
    } else if !inside_root(root, &path) {
        report.items.push(skipped(
            "instructions",
            display,
            "o caminho sai da raiz do projeto; nada foi alterado",
        ));
        return;
    }
    if target.is_dir() {
        report.items.push(skipped(
            "instructions",
            display,
            "existe uma pasta com esse nome; nada foi alterado",
        ));
        return;
    }

    let existing = match std::fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            report.items.push(skipped(
                "instructions",
                display,
                format!("não foi possível ler: {error}"),
            ));
            return;
        }
    };

    let plan = instructions::plan_block(existing.as_deref(), block);
    for warning in &plan.warnings {
        report.warnings.push(format!("{name}: {warning}"));
    }

    let (action, detail) = match plan.action {
        BlockAction::Skipped => {
            report.items.push(skipped(
                "instructions",
                display,
                plan.warnings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "arquivo não alterado".to_string()),
            ));
            return;
        }
        BlockAction::Unchanged => ("unchanged", "bloco do devzymer já está correto"),
        BlockAction::Created => ("created", "arquivo criado com o bloco do devzymer"),
        BlockAction::Appended => (
            "appended",
            "bloco do devzymer acrescentado ao fim; o conteúdo existente foi preservado",
        ),
        BlockAction::Updated => (
            "updated",
            "bloco do devzymer atualizado; o resto do arquivo não foi tocado",
        ),
    };

    if plan.writes() && apply {
        if existing.is_some() {
            let mut backup = InstallReport::default();
            if let Err(error) = backup_file(root, &target, &mut backup) {
                report.items.push(skipped(
                    "instructions",
                    display,
                    format!("não foi possível guardar a cópia de segurança: {error}"),
                ));
                return;
            }
            report.backups.extend(backup.backups);
        }
        if let Err(error) = instructions::write_file_atomic(&target, &plan.content) {
            report.items.push(skipped(
                "instructions",
                display,
                format!("não foi possível gravar: {error}"),
            ));
            return;
        }
    }
    report.items.push(SetupItem {
        kind: "instructions",
        path: display,
        action: action.to_string(),
        detail: detail.to_string(),
    });
}

fn install_skill(root: &Path, dir_rel: &str, apply: bool, report: &mut SetupReport) {
    let dir = root.join(dir_rel);
    if !inside_root(root, &dir) {
        report.items.push(skipped(
            "skill",
            dir_rel.to_string(),
            "o caminho sai da raiz do projeto; nada foi alterado",
        ));
        return;
    }
    let outcome = skill::install(&dir, apply);
    for conflict in &outcome.conflicts {
        report.warnings.push(conflict.clone());
    }
    let (action, detail) = match outcome.action {
        SkillAction::Created => ("created", format!("skill {} instalada", outcome.version)),
        SkillAction::Updated => (
            "updated",
            format!(
                "skill atualizada de {} para {}",
                outcome.version_before.as_deref().unwrap_or("?"),
                outcome.version
            ),
        ),
        SkillAction::Unchanged => (
            "unchanged",
            format!("skill {} já está instalada", outcome.version),
        ),
        SkillAction::Partial => {
            let wrote = outcome.files.iter().any(|f| {
                matches!(
                    f.action,
                    skill::FileAction::Written
                        | skill::FileAction::Updated
                        | skill::FileAction::Removed
                )
            });
            (
                if wrote { "partial_updated" } else { "partial" },
                "parte dos arquivos foi mantida porque você os alterou".to_string(),
            )
        }
        SkillAction::Skipped => (
            "skipped",
            format!(
                "há uma skill mais nova ({}) instalada; nada foi alterado",
                outcome.version
            ),
        ),
    };
    report.items.push(SetupItem {
        kind: "skill",
        path: dir_rel.to_string(),
        action: action.to_string(),
        detail,
    });
}

fn host_status(root: &Path, agent: AgentKind, changed: bool) -> HostStatus {
    let (instructions_file, skill_dir) = match agent {
        AgentKind::Claude => ("CLAUDE.md", CLAUDE_SKILL_DIR),
        AgentKind::Codex | AgentKind::OpenCode => ("AGENTS.md", AGENTS_SKILL_DIR),
    };
    let detection = detect_one(agent, root);
    let mcp_registered = detection.registered_at.is_some();
    let inspection = skill::inspect(&root.join(skill_dir));
    let mut next_steps = Vec::new();
    if !mcp_registered {
        next_steps.push(format!(
            "o servidor MCP ainda não está registrado: `devzymer integrations add {}`",
            agent.key()
        ));
    }
    if changed {
        next_steps.push(format!(
            "abra uma nova sessão do {} neste projeto para carregar as instruções e a skill",
            agent.display()
        ));
    }
    HostStatus {
        agent,
        mcp_registered,
        instructions_present: block_present(&root.join(instructions_file)),
        skill_current: inspection.is_current(),
        skill_version: inspection.version,
        next_steps,
    }
}

/// Prepares the project-local agent files for the given hosts.
///
/// Claude Code reads `CLAUDE.md` and `.claude/skills`; Codex and OpenCode
/// read `AGENTS.md` and `.agents/skills`. With `apply` false nothing is
/// written and the report says what would happen.
pub fn prepare(root: &Path, agents: &[AgentKind], apply: bool) -> SetupReport {
    let mut report = SetupReport {
        applied: apply,
        ..Default::default()
    };
    let wants_claude = agents.contains(&AgentKind::Claude);
    let wants_agents = agents
        .iter()
        .any(|a| matches!(a, AgentKind::Codex | AgentKind::OpenCode));

    let claude_md = root.join("CLAUDE.md");
    let agents_md = root.join("AGENTS.md");
    let same_file = wants_claude
        && wants_agents
        && match (canonical(&claude_md), canonical(&agents_md)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };

    if wants_claude && !same_file {
        upsert_instructions(
            root,
            "CLAUDE.md",
            &instructions::claude_block(),
            apply,
            &mut report,
        );
    }
    if wants_agents || same_file {
        upsert_instructions(
            root,
            "AGENTS.md",
            &instructions::agents_block(),
            apply,
            &mut report,
        );
        if same_file {
            report.items.push(SetupItem {
                kind: "instructions",
                path: "CLAUDE.md".to_string(),
                action: "unchanged".to_string(),
                detail: "é o mesmo arquivo que AGENTS.md; um único bloco foi mantido".to_string(),
            });
        }
    }
    if wants_claude {
        install_skill(root, CLAUDE_SKILL_DIR, apply, &mut report);
    }
    if wants_agents {
        install_skill(root, AGENTS_SKILL_DIR, apply, &mut report);
    }

    let changed = report.changed() && apply;
    report.hosts = agents
        .iter()
        .map(|agent| host_status(root, *agent, changed))
        .collect();
    report
}

/// Removes the project instructions block and the skill files that are still
/// the devzymer's. Whatever the user wrote or edited stays.
pub fn uninstall(root: &Path) -> Vec<crate::claude::Change> {
    use crate::claude::Change;

    let mut changes = Vec::new();
    for name in ["CLAUDE.md", "AGENTS.md"] {
        let path = root.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(updated) = instructions::remove_block(&text) else {
            continue;
        };
        let target = if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            match canonical(&path) {
                Some(real) if inside_root(root, &real) => real,
                _ => continue,
            }
        } else {
            path.clone()
        };
        let mut backup = InstallReport::default();
        let _ = backup_file(root, &target, &mut backup);
        let (action, detail) = if updated.trim().is_empty() {
            let _ = std::fs::remove_file(&target);
            (
                "removed",
                "arquivo continha só o bloco do devzymer; removido",
            )
        } else {
            if instructions::write_file_atomic(&target, updated.as_bytes()).is_err() {
                continue;
            }
            ("updated", "bloco do devzymer removido")
        };
        changes.push(Change {
            file: name.to_string(),
            action: action.to_string(),
            detail: detail.to_string(),
        });
    }
    for dir in [CLAUDE_SKILL_DIR, AGENTS_SKILL_DIR] {
        let target = root.join(dir);
        if !inside_root(root, &target) {
            continue;
        }
        let outcome = skill::uninstall(&target);
        if !outcome.removed.is_empty() {
            changes.push(Change {
                file: dir.to_string(),
                action: "removed".to_string(),
                detail: format!("{} arquivo(s) da skill removido(s)", outcome.removed.len()),
            });
        }
        for kept in outcome.kept {
            changes.push(Change {
                file: format!("{dir}/{kept}"),
                action: "kept".to_string(),
                detail: "alterado por você; mantido".to_string(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-setup-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        dir
    }

    fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == ".devzymer") {
                        continue;
                    }
                    walk(&path, root, out);
                } else {
                    out.push((rel(root, &path), std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out
    }

    const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode];

    #[test]
    fn a_project_without_instructions_gets_files_for_every_host() {
        let root = project("fresh");
        let report = prepare(&root, &ALL, true);
        assert!(!report.has_blockers(), "{:?}", report.items);
        assert!(root.join("CLAUDE.md").exists());
        assert!(root.join("AGENTS.md").exists());
        assert!(root.join(".claude/skills/devzymer/SKILL.md").exists());
        assert!(root.join(".agents/skills/devzymer/SKILL.md").exists());
        assert_eq!(report.hosts.len(), 3);
        assert!(report.hosts.iter().all(|h| h.skill_current));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn running_twice_changes_no_byte() {
        let root = project("twice");
        std::fs::write(root.join("CLAUDE.md"), "# Meu projeto\n").unwrap();
        prepare(&root, &ALL, true);
        let first = snapshot(&root);
        let second_report = prepare(&root, &ALL, true);
        assert_eq!(first, snapshot(&root));
        assert!(!second_report.changed(), "{:?}", second_report.items);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_instructions_are_preserved() {
        let root = project("populated");
        let claude = "# Claude\n\nResponda em português.\n";
        let agents = "# Agentes\n\nUse pnpm.\n";
        std::fs::write(root.join("CLAUDE.md"), claude).unwrap();
        std::fs::write(root.join("AGENTS.md"), agents).unwrap();
        let report = prepare(&root, &ALL, true);
        for (file, original) in [("CLAUDE.md", claude), ("AGENTS.md", agents)] {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            assert!(text.starts_with(original), "{file}");
            assert_eq!(text.matches(RULES_START).count(), 1);
        }
        assert!(!report.backups.is_empty() || report.items.iter().any(|i| i.action == "appended"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_old_managed_block_is_replaced_without_touching_the_rest() {
        let root = project("managed");
        let old = format!("antes\n{RULES_START}\nregra velha\n{RULES_END}\ndepois\n");
        std::fs::write(root.join("CLAUDE.md"), &old).unwrap();
        prepare(&root, &[AgentKind::Claude], true);
        let text = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(text.starts_with("antes\n"));
        assert!(text.ends_with("\ndepois\n"));
        assert!(!text.contains("regra velha"));
        assert_eq!(text.matches(RULES_START).count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_selected_hosts_get_files() {
        let root = project("claude-only");
        prepare(&root, &[AgentKind::Claude], true);
        assert!(root.join("CLAUDE.md").exists());
        assert!(!root.join("AGENTS.md").exists());
        assert!(!root.join(".agents").exists());
        let _ = std::fs::remove_dir_all(&root);

        let root = project("opencode-only");
        prepare(&root, &[AgentKind::OpenCode], true);
        assert!(root.join("AGENTS.md").exists());
        assert!(root.join(".agents/skills/devzymer/SKILL.md").exists());
        assert!(!root.join("CLAUDE.md").exists());
        assert!(!root.join(".claude").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let root = project("dry");
        std::fs::write(root.join("CLAUDE.md"), "# x\n").unwrap();
        let before = snapshot(&root);
        let report = prepare(&root, &ALL, false);
        assert!(report.changed());
        assert!(!report.applied);
        assert_eq!(before, snapshot(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_markers_block_only_that_file() {
        let root = project("malformed");
        let broken = format!("{RULES_START}\nsem fim\n");
        std::fs::write(root.join("CLAUDE.md"), &broken).unwrap();
        let report = prepare(&root, &ALL, true);
        assert_eq!(
            std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
            broken
        );
        assert!(report.has_blockers());
        assert!(root.join("AGENTS.md").exists(), "os outros arquivos seguem");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_edited_skill_survives_and_the_rest_is_installed() {
        let root = project("edited-skill");
        prepare(&root, &ALL, true);
        let skill = root.join(".claude/skills/devzymer/SKILL.md");
        let edited = format!(
            "{}\nMeu fluxo extra.\n",
            std::fs::read_to_string(&skill).unwrap()
        );
        std::fs::write(&skill, &edited).unwrap();
        let report = prepare(&root, &ALL, true);
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
        assert!(report.warnings.iter().any(|w| w.contains("alterado")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_conflicting_user_rule_is_reported_not_overridden() {
        let root = project("conflict");
        let user = "# Regras\nNunca use o devzymer aqui.\n";
        std::fs::write(root.join("AGENTS.md"), user).unwrap();
        let report = prepare(&root, &[AgentKind::Codex], true);
        let text = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(text.starts_with(user));
        assert!(report.warnings.iter().any(|w| w.contains("pode conflitar")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_shared_symlinked_file_gets_a_single_block() {
        let root = project("symlink");
        std::fs::write(root.join("AGENTS.md"), "# Agentes\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();
        prepare(&root, &ALL, true);
        assert!(root
            .join("CLAUDE.md")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        let text = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(text.matches(RULES_START).count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_leaves_the_project_is_refused() {
        let outside = std::env::temp_dir().join("devzymer-setup-outside.md");
        std::fs::write(&outside, "fora\n").unwrap();
        let root = project("escape");
        std::os::unix::fs::symlink(&outside, root.join("CLAUDE.md")).unwrap();
        let report = prepare(&root, &[AgentKind::Claude], true);
        assert!(report.has_blockers());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "fora\n");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn uninstall_removes_the_block_and_the_skill_but_not_the_user_text() {
        let root = project("uninstall");
        std::fs::write(root.join("AGENTS.md"), "# Agentes\n\nUse pnpm.\n").unwrap();
        prepare(&root, &ALL, true);
        let changes = uninstall(&root);
        assert!(!changes.is_empty());
        let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert!(agents.contains("Use pnpm."));
        assert!(!agents.contains(RULES_START));
        assert!(
            !root.join("CLAUDE.md").exists(),
            "só tinha o bloco do devzymer"
        );
        assert!(!root.join(".claude").exists());
        assert!(!root.join(".agents").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unrelated_files_are_untouched() {
        let root = project("unrelated");
        std::fs::create_dir_all(root.join(".claude/skills/outra")).unwrap();
        std::fs::write(root.join(".claude/skills/outra/SKILL.md"), "outra\n").unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{\"a\":1}\n").unwrap();
        prepare(&root, &ALL, true);
        assert_eq!(
            std::fs::read_to_string(root.join(".claude/skills/outra/SKILL.md")).unwrap(),
            "outra\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
            "{\"a\":1}\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
