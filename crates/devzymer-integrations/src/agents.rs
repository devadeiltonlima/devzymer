//! Detecção e instalação compartilhada entre os agentes.
//!
//! Cada agente tem um arquivo de configuração próprio, mas o motor é um só: o
//! mesmo índice local, o mesmo grafo e o mesmo servidor MCP servem todos.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;
use serde::Serialize;

use crate::claude::{Change, InstallReport, InstallScope};

/// Os agentes que o devzymer sabe integrar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum AgentKind {
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl AgentKind {
    /// Todos os agentes suportados, na ordem de exibição.
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode];

    /// Interpreta o nome usado na linha de comando.
    pub fn parse(value: &str) -> Option<AgentKind> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Some(AgentKind::Claude),
            "codex" | "openai-codex" | "oai-codex" => Some(AgentKind::Codex),
            "opencode" | "open-code" => Some(AgentKind::OpenCode),
            _ => None,
        }
    }

    /// Interpreta uma lista separada por vírgula.
    pub fn parse_list(value: &str) -> Result<Vec<AgentKind>> {
        let mut out: Vec<AgentKind> = Vec::new();
        for part in value.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let kind = AgentKind::parse(part).ok_or_else(|| {
                anyhow::anyhow!("agente desconhecido '{part}'. Use claude, codex ou opencode.")
            })?;
            if !out.contains(&kind) {
                out.push(kind);
            }
        }
        if out.is_empty() {
            anyhow::bail!("nenhum agente informado. Use claude, codex ou opencode.");
        }
        Ok(out)
    }

    /// O nome exibido ao usuário.
    pub fn display(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::OpenCode => "OpenCode",
        }
    }

    /// A chave usada na linha de comando.
    pub fn key(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
        }
    }

    /// The executable that proves the agent is installed.
    pub fn executable_name(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
        }
    }
}

/// O estado de um agente, do mais forte para o mais fraco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// A configuração existe e o servidor respondeu a um handshake real.
    Connected,
    /// A configuração aponta para o devzymer.
    Configured,
    /// O executável do agente existe.
    Detected,
    /// Nada foi encontrado.
    NotInstalled,
}

impl AgentStatus {
    pub fn display(self) -> &'static str {
        match self {
            AgentStatus::Connected => "Conectado",
            AgentStatus::Configured => "Configurado",
            AgentStatus::Detected => "Detectado",
            AgentStatus::NotInstalled => "Não instalado",
        }
    }
}

/// O que foi encontrado sobre um agente.
#[derive(Debug, Clone, Serialize)]
pub struct AgentDetection {
    pub agent: AgentKind,
    pub status: AgentStatus,
    pub executable: Option<PathBuf>,
    /// Onde a configuração do agente vive.
    pub config_path: Option<PathBuf>,
    /// Onde o devzymer está registrado, quando está.
    pub registered_at: Option<PathBuf>,
    pub notes: Vec<String>,
}

impl AgentDetection {
    /// True when the agent can receive an integration.
    pub fn is_available(&self) -> bool {
        self.executable.is_some() || self.registered_at.is_some()
    }
}

/// Detecta os três agentes no ambiente.
pub fn detect(root: &Path) -> Vec<AgentDetection> {
    AgentKind::ALL
        .iter()
        .map(|kind| detect_one(*kind, root))
        .collect()
}

/// Detecta um agente.
pub fn detect_one(kind: AgentKind, root: &Path) -> AgentDetection {
    let executable = devzymer_core::workspace::which(kind.executable_name());
    let (config_path, registered_at) = match kind {
        AgentKind::Claude => claude_locations(root),
        AgentKind::Codex => codex_locations(root),
        AgentKind::OpenCode => opencode_locations(root),
    };
    let mut notes = Vec::new();
    if executable.is_none() {
        notes.push(format!(
            "executável '{}' não encontrado no PATH",
            kind.executable_name()
        ));
    }
    let status = if registered_at.is_some() {
        AgentStatus::Configured
    } else if executable.is_some() {
        AgentStatus::Detected
    } else {
        AgentStatus::NotInstalled
    };
    AgentDetection {
        agent: kind,
        status,
        executable,
        config_path,
        registered_at,
        notes,
    }
}

fn claude_locations(root: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let project = root.join(".mcp.json");
    let user = devzymer_core::paths::user_home().join(".claude.json");
    let mut config = None;
    let mut registered = None;
    for candidate in [project, user] {
        if !candidate.exists() {
            continue;
        }
        if config.is_none() {
            config = Some(candidate.clone());
        }
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        if text.contains("\"devzymer\"") {
            registered = Some(candidate);
            break;
        }
    }
    (config, registered)
}

fn codex_locations(root: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let project = crate::codex::project_config_path(root);
    let user = crate::codex::user_config_path();
    let mut config = None;
    let mut registered = None;
    for candidate in [project, user] {
        if !candidate.exists() {
            continue;
        }
        if config.is_none() {
            config = Some(candidate.clone());
        }
        if crate::codex::is_installed(root, scope_of(&candidate, root)) {
            registered = Some(candidate);
            break;
        }
    }
    (config, registered)
}

fn scope_of(candidate: &Path, root: &Path) -> InstallScope {
    if candidate.starts_with(root) {
        InstallScope::Project
    } else {
        InstallScope::User
    }
}

fn opencode_locations(root: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let project = crate::opencode::project_config_path(root);
    let user = crate::opencode::config_path(root, InstallScope::User);
    let mut config = None;
    let mut registered = None;
    for candidate in [project, user] {
        if !candidate.exists() {
            continue;
        }
        if config.is_none() {
            config = Some(candidate.clone());
        }
        if crate::opencode::is_installed(root, scope_of(&candidate, root)) {
            registered = Some(candidate);
            break;
        }
    }
    (config, registered)
}

/// O resultado da instalação de um agente.
#[derive(Debug, Clone, Serialize)]
pub struct AgentOutcome {
    pub agent: AgentKind,
    pub scope: InstallScope,
    pub ok: bool,
    pub verified: bool,
    pub detail: String,
    pub changes: Vec<Change>,
}

/// Instala a integração nos agentes selecionados.
///
/// Cada agente é tratado por conta própria: uma falha em um não desfaz os que
/// já foram instalados.
pub fn install(
    root: &Path,
    config: &devzymer_core::config::Config,
    agents: &[AgentKind],
    scope: InstallScope,
    executable: &Path,
    server_only: bool,
) -> Result<Vec<AgentOutcome>> {
    install_with_rules(root, config, agents, scope, executable, server_only, true)
}

/// Como [`install`], com controle sobre as instruções do agente.
///
/// Com `rules` falso, o arquivo de instruções não é escrito; o registro do
/// servidor, as permissões e os hooks seguem as demais opções.
pub fn install_with_rules(
    root: &Path,
    config: &devzymer_core::config::Config,
    agents: &[AgentKind],
    scope: InstallScope,
    executable: &Path,
    server_only: bool,
    rules: bool,
) -> Result<Vec<AgentOutcome>> {
    let mut out = Vec::new();
    for agent in agents {
        let mut report = InstallReport {
            scope: scope.display().to_string(),
            server_name: "devzymer".to_string(),
            command: executable.display().to_string(),
            ..Default::default()
        };
        let outcome = match install_one(
            root,
            config,
            *agent,
            scope,
            executable,
            server_only,
            rules,
            &mut report,
        ) {
            Ok(()) => {
                let verified = probe_server(executable, root);
                AgentOutcome {
                    agent: *agent,
                    scope,
                    ok: true,
                    verified,
                    detail: if verified {
                        "servidor MCP verificado".to_string()
                    } else {
                        "configuração gravada, mas o handshake MCP não respondeu".to_string()
                    },
                    changes: report.changes,
                }
            }
            Err(error) => AgentOutcome {
                agent: *agent,
                scope,
                ok: false,
                verified: false,
                detail: error.to_string(),
                changes: report.changes,
            },
        };
        out.push(outcome);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn install_one(
    root: &Path,
    config: &devzymer_core::config::Config,
    agent: AgentKind,
    scope: InstallScope,
    executable: &Path,
    server_only: bool,
    rules: bool,
    report: &mut InstallReport,
) -> Result<()> {
    match agent {
        AgentKind::Claude => {
            if server_only {
                crate::claude::install_server(root, config, scope, Some(executable.to_path_buf()))?;
            } else if !rules {
                crate::claude::install_without_rules(
                    root,
                    config,
                    scope,
                    Some(executable.to_path_buf()),
                )?;
            } else {
                crate::claude::install(root, config, scope, Some(executable.to_path_buf()))?;
            }
            Ok(())
        }
        AgentKind::Codex => {
            crate::codex::install(root, scope, executable, report)?;
            if !server_only {
                if rules {
                    install_rules(root, scope, report, crate::codex::rules_path(root, scope))?;
                }
                crate::codex::install_hooks(root, scope, executable, report)?;
            }
            Ok(())
        }
        AgentKind::OpenCode => {
            crate::opencode::install(root, scope, executable, report)?;
            if !server_only {
                if rules {
                    install_rules(
                        root,
                        scope,
                        report,
                        crate::opencode::rules_path(root, scope),
                    )?;
                }
                crate::opencode::install_plugin(
                    root,
                    scope,
                    executable,
                    crate::opencode::installed_major(),
                    report,
                )?;
            }
            Ok(())
        }
    }
}

/// Escreve o bloco de regras do devzymer no arquivo de instruções do agente.
fn install_rules(
    root: &Path,
    scope: InstallScope,
    report: &mut InstallReport,
    target: PathBuf,
) -> Result<()> {
    let _ = scope;
    crate::claude::write_rules_block(&target, root, &crate::claude::rules_block_generic(), report)
        .map_err(anyhow::Error::from)
}

/// Remove a integração do devzymer de um agente.
pub fn uninstall_agent(
    root: &Path,
    agent: AgentKind,
    scope: InstallScope,
) -> Result<InstallReport> {
    let mut report = InstallReport {
        scope: scope.display().to_string(),
        server_name: "devzymer".to_string(),
        ..Default::default()
    };
    match agent {
        AgentKind::Claude => {
            crate::claude::uninstall(root, "devzymer", scope)?;
        }
        AgentKind::Codex => {
            crate::codex::uninstall(root, scope, &mut report)?;
            crate::codex::uninstall_hooks(root, scope, &mut report)?;
        }
        AgentKind::OpenCode => {
            crate::opencode::uninstall(root, scope, &mut report)?;
            crate::opencode::uninstall_plugin(root, scope, &mut report)?;
        }
    }
    Ok(report)
}

/// Verifica o servidor MCP com um handshake real.
///
/// A resposta do servidor é lida do processo, então um binário quebrado ou um
/// índice inacessível aparecem como falha, e não como sucesso presumida.
pub fn probe_server(executable: &Path, root: &Path) -> bool {
    use std::io::{BufRead, BufReader, Write};

    let Ok(mut child) = Command::new(executable)
        .arg("serve")
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let Some(stdin) = child.stdin.as_mut() else {
        let _ = child.kill();
        return false;
    };
    let Some(stdout) = child.stdout.as_mut() else {
        let _ = child.kill();
        return false;
    };
    let mut reader = BufReader::new(stdout);
    let mut answer = String::new();

    for request in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ] {
        if stdin.write_all(request.as_bytes()).is_err() || stdin.write_all(b"\n").is_err() {
            let _ = child.kill();
            return false;
        }
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            let _ = child.kill();
            return false;
        }
        answer.push_str(&line);
    }
    let _ = child.kill();
    let _ = child.wait();
    answer.contains("\"result\"") && answer.contains("context_resolve")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_list_is_parsed() {
        let list = AgentKind::parse_list("claude,codex").unwrap();
        assert_eq!(list, vec![AgentKind::Claude, AgentKind::Codex]);
        let single = AgentKind::parse_list("opencode").unwrap();
        assert_eq!(single, vec![AgentKind::OpenCode]);
    }

    #[test]
    fn an_unknown_agent_is_rejected() {
        assert!(AgentKind::parse_list("claude,emacs").is_err());
        assert!(AgentKind::parse_list("").is_err());
    }

    #[test]
    fn a_repeated_agent_is_listed_once() {
        let list = AgentKind::parse_list("codex,codex").unwrap();
        assert_eq!(list, vec![AgentKind::Codex]);
    }

    #[test]
    fn every_status_has_a_label() {
        for status in [
            AgentStatus::Connected,
            AgentStatus::Configured,
            AgentStatus::Detected,
            AgentStatus::NotInstalled,
        ] {
            assert!(!status.display().is_empty());
        }
    }
}
