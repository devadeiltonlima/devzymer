use std::path::{Path, PathBuf};

use devzymer_core::config::write_atomic;
use devzymer_core::config::Config;
use devzymer_core::Result;
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallScope {
    /// Shared with the team through the repository `.mcp.json`.
    Project,
    /// Stored in `~/.claude.json` for the current project only.
    Local,
    /// Stored in `~/.claude.json` and shared across every project.
    User,
}

impl InstallScope {
    pub fn parse(value: &str) -> Option<InstallScope> {
        match value.to_ascii_lowercase().as_str() {
            "project" => Some(InstallScope::Project),
            "local" => Some(InstallScope::Local),
            "user" => Some(InstallScope::User),
            _ => None,
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            InstallScope::Project => "project",
            InstallScope::Local => "local",
            InstallScope::User => "user",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Change {
    pub file: String,
    pub action: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct InstallReport {
    pub scope: String,
    pub changes: Vec<Change>,
    pub backups: Vec<String>,
    pub warnings: Vec<String>,
    pub server_name: String,
    pub command: String,
}

pub const RULES_START: &str = "<!-- devzymer:start -->";
pub const RULES_END: &str = "<!-- devzymer:end -->";

/// Installs the MCP registration, the agent rules and the supported hooks.
pub fn install(
    root: &Path,
    config: &Config,
    scope: InstallScope,
    command: Option<PathBuf>,
) -> Result<InstallReport> {
    install_with(root, config, scope, command, false)
}

/// Installs only the MCP registration, leaving the rules and the hooks
/// untouched. Useful for the user scope, where the rules would otherwise apply
/// to every project of the machine.
pub fn install_server(
    root: &Path,
    config: &Config,
    scope: InstallScope,
    command: Option<PathBuf>,
) -> Result<InstallReport> {
    install_with(root, config, scope, command, true)
}

fn install_with(
    root: &Path,
    config: &Config,
    scope: InstallScope,
    command: Option<PathBuf>,
    server_only: bool,
) -> Result<InstallReport> {
    let executable = match command {
        Some(path) => path,
        None => std::env::current_exe()
            .map_err(|e| devzymer_core::Error::io("current executable", e))?,
    };
    let server_name = config.mcp.server_name.clone();
    let mut report = InstallReport {
        scope: scope.display().to_string(),
        server_name: server_name.clone(),
        command: executable.display().to_string(),
        ..Default::default()
    };

    let (rules_path, settings_dir) = scope_paths(root, scope);

    match scope {
        InstallScope::Project => {
            let target = root.join(".mcp.json");
            install_mcp_entry(
                root,
                &target,
                &["mcpServers", server_name.as_str()],
                &server_entry(&executable, &server_name),
                &mut report,
            )?;
        }
        InstallScope::Local => {
            let target = devzymer_core::paths::user_home().join(".claude.json");
            let project_key = project_key(root);
            let path = vec![
                "projects",
                project_key.as_str(),
                "mcpServers",
                server_name.as_str(),
            ];
            install_mcp_entry(
                root,
                &target,
                &path,
                &server_entry(&executable, &server_name),
                &mut report,
            )?;
        }
        InstallScope::User => {
            let target = devzymer_core::paths::user_home().join(".claude.json");
            install_mcp_entry(
                root,
                &target,
                &["mcpServers", server_name.as_str()],
                &server_entry(&executable, &server_name),
                &mut report,
            )?;
        }
    }

    install_permissions(root, &settings_dir, &server_name, &mut report)?;
    ensure_gitignore(root, &mut report)?;

    if !server_only {
        install_rules(&rules_path, root, &mut report)?;
        install_hooks(root, &executable, &settings_dir, &mut report)?;
    }

    Ok(report)
}

/// Key used by Claude Code to identify a project inside `~/.claude.json`.
/// Claude Code stores the path with forward slashes, so the key must use the
/// same form or the entry is never found again.
fn project_key(root: &Path) -> String {
    devzymer_core::util::canonical_display(root)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn server_entry(executable: &Path, server_name: &str) -> Value {
    json!({
        "type": "stdio",
        "command": executable.display().to_string(),
        "args": ["serve", "--root", "${CLAUDE_PROJECT_DIR:-.}"]
    })
    .as_object()
    .cloned()
    .map(|mut map| {
        map.insert("__devzymer".to_string(), json!(server_name));
        Value::Object(map)
    })
    .unwrap_or(Value::Null)
}

fn install_mcp_entry(
    root: &Path,
    target: &Path,
    path: &[&str],
    entry: &Value,
    report: &mut InstallReport,
) -> Result<()> {
    let mut document = read_json(target)?;
    let mut clean_entry = entry.clone();
    if let Some(map) = clean_entry.as_object_mut() {
        map.remove("__devzymer");
    }

    let before = document.to_string();
    set_json_path(&mut document, path, clean_entry.clone());

    if before == document {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "a entrada do servidor MCP já está correta".into(),
        });
        return Ok(());
    }

    backup_file(root, target, report)?;
    write_json(target, &document)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: format!(
            "registrado o servidor MCP '{}' em {}",
            path.last().unwrap_or(&""),
            path.join(".")
        ),
    });
    Ok(())
}

/// Applies a change to a JSON file that another process may be writing, such
/// as `~/.claude.json`. The file is re-read and the change re-applied when the
/// content changed in the meantime, so a concurrent write is not lost.
fn update_json_file<F>(path: &Path, mut mutator: F) -> Result<bool>
where
    F: FnMut(&mut Value) -> bool,
{
    for _ in 0..5 {
        let original = std::fs::read_to_string(path).unwrap_or_default();
        let mut document: Value = if original.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&original).unwrap_or_else(|_| json!({}))
        };
        let snapshot = document.clone();
        if !mutator(&mut document) {
            return Ok(false);
        }
        if document == snapshot {
            return Ok(false);
        }
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current != original {
            continue;
        }
        write_json(path, &document)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn install_rules(rules_path: &Path, root: &Path, report: &mut InstallReport) -> Result<()> {
    let target = rules_path.to_path_buf();
    let rules = rules_block();
    let existing = std::fs::read_to_string(&target).unwrap_or_default();

    // The old block is removed and the new one goes first, so the rules take
    // precedence over the instructions of any other indexing tool configured
    // in the project.
    let mut without = existing.clone();
    if let Some(start) = without.find(RULES_START) {
        let end = without
            .find(RULES_END)
            .map(|e| e + RULES_END.len())
            .unwrap_or(without.len());
        without = format!("{}{}", &without[..start], &without[end..]);
    }
    let mut updated = rules.clone();
    if !updated.ends_with("\n\n") {
        updated.push('\n');
    }
    updated.push_str(without.trim_start_matches('\n'));

    if updated == existing {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "as regras do devzymer já estão instaladas".into(),
        });
        return Ok(());
    }

    if !existing.is_empty() {
        backup_file(root, &target, report)?;
    }
    write_atomic(&target, updated.as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "regras de uso do devzymer instaladas".into(),
    });
    Ok(())
}

fn install_hooks(
    root: &Path,
    executable: &Path,
    settings_dir: &Path,
    report: &mut InstallReport,
) -> Result<()> {
    let target = settings_dir.join("settings.json");
    let post_use = json!({
        "matcher": "Bash",
        "hooks": [{
            "type": "command",
            "command": executable.display().to_string(),
            "args": ["hook", "post-tool-use"],
            "timeout": 20
        }]
    });
    let pre_use = json!({
        "matcher": "Write|Edit|MultiEdit|NotebookEdit|Bash",
        "hooks": [{
            "type": "command",
            "command": executable.display().to_string(),
            "args": ["hook", "guard-test-files"],
            "timeout": 10
        }]
    });
    let guard_search = json!({
        "matcher": "Grep|Glob|Read",
        "hooks": [{
            "type": "command",
            "command": executable.display().to_string(),
            "args": ["hook", "guard-search"],
            "timeout": 10
        }]
    });

    let changed = update_json_file(&target, |document| {
        let Some(hooks) = document
            .as_object_mut()
            .map(|map| map.entry("hooks").or_insert_with(|| json!({})))
        else {
            return false;
        };
        merge_hook_group(hooks, "PostToolUse", &post_use)
            | merge_hook_group(hooks, "PreToolUse", &pre_use)
            | merge_hook_group(hooks, "PreToolUse", &guard_search)
    })?;

    if !changed {
        report.changes.push(Change {
            file: target.display().to_string(),
            action: "unchanged".into(),
            detail: "os hooks do devzymer já estão instalados".into(),
        });
        return Ok(());
    }

    backup_file(root, &target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "hooks de compressão de saída e de guarda de arquivos de teste instalados".into(),
    });
    Ok(())
}

fn merge_hook_group(hooks: &mut Value, event: &str, group: &Value) -> bool {
    let Some(hooks_map) = hooks.as_object_mut() else {
        return false;
    };
    let entry = hooks_map
        .entry(event.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(list) = entry.as_array_mut() else {
        *entry = Value::Array(Vec::new());
        return true;
    };

    // A previous installation with a different binary path is replaced, not
    // duplicated. Only the group of the same matcher is replaced, so the
    // guards of the same event coexist.
    let matcher = group
        .get("matcher")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string();
    let previous = list.len();
    list.retain(|existing| !group_is_devzymer(existing) || group_matcher(existing) != matcher);
    let replaced = previous != list.len();

    let command = group
        .get("hooks")
        .and_then(|h| h.get(0))
        .and_then(|h| h.get("command"))
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();

    for existing in list.iter() {
        let same = existing
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|handlers| {
                handlers.iter().any(|handler| {
                    handler.get("command").and_then(|c| c.as_str()) == Some(command.as_str())
                        && handler.get("args")
                            == group
                                .get("hooks")
                                .and_then(|h| h.get(0))
                                .and_then(|h| h.get("args"))
                })
            })
            .unwrap_or(false);
        if same {
            return replaced;
        }
    }
    list.push(group.clone());
    true
}

/// The matcher of a hook group.
fn group_matcher(group: &Value) -> String {
    group
        .get("matcher")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string()
}

/// True when a hook group was installed by devzymer, whatever binary path it
/// points to.
fn group_is_devzymer(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|handlers| handlers.iter().any(handler_is_devzymer))
        .unwrap_or(false)
}

fn handler_is_devzymer(handler: &Value) -> bool {
    let command = handler
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if command.contains("devzymer") {
        return true;
    }
    handler
        .get("args")
        .and_then(|a| a.as_array())
        .map(|args| args.iter().any(|a| a.as_str() == Some("hook")))
        .unwrap_or(false)
}

/// Removes only the entries owned by devzymer.
pub fn uninstall(root: &Path, server_name: &str, scope: InstallScope) -> Result<InstallReport> {
    let mut report = InstallReport {
        scope: scope.display().to_string(),
        server_name: server_name.to_string(),
        ..Default::default()
    };

    match scope {
        InstallScope::Project => {
            let target = root.join(".mcp.json");
            remove_mcp_entry(root, &target, &["mcpServers", server_name], &mut report)?;
        }
        InstallScope::Local => {
            let target = devzymer_core::paths::user_home().join(".claude.json");
            let key = project_key(root);
            let path = vec!["projects", key.as_str(), "mcpServers", server_name];
            remove_mcp_entry(root, &target, &path, &mut report)?;
        }
        InstallScope::User => {
            let target = devzymer_core::paths::user_home().join(".claude.json");
            remove_mcp_entry(root, &target, &["mcpServers", server_name], &mut report)?;
        }
    }

    let (rules_path, settings_dir) = scope_paths(root, scope);
    remove_rules(&rules_path, root, &mut report)?;
    remove_hooks(&settings_dir, root, &mut report)?;
    remove_permissions(root, &settings_dir, server_name, &mut report)?;
    Ok(report)
}

/// Keeps the local index out of the version control.
///
/// The index is a machine specific SQLite file. Committing it adds noise to
/// the history and creates conflicts that no one can resolve.
fn ensure_gitignore(root: &Path, report: &mut InstallReport) -> Result<()> {
    if !root.join(".git").exists() {
        return Ok(());
    }
    let target = root.join(".gitignore");
    let existing = std::fs::read_to_string(&target).unwrap_or_default();
    if gitignore_covers(&existing, ".devzymer") {
        return Ok(());
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    if !content.is_empty() && !content.ends_with("\n\n") {
        content.push('\n');
    }
    content.push_str("# devzymer\n.devzymer/\n");

    std::fs::write(&target, &content).map_err(|e| devzymer_core::Error::io(&target, e))?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "indice local (.devzymer/) incluido no .gitignore".into(),
    });
    Ok(())
}

/// True when the rules of a `.gitignore` already cover the given path.
fn gitignore_covers(content: &str, name: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            return false;
        }
        let line = line.trim_end_matches('/');
        let line = line.strip_prefix('/').unwrap_or(line);
        line == name
    })
}

/// Grants the agent the permission to call the tools of the server.
///
/// Without the rule, every call is rejected with "you haven't granted it yet",
/// and the agent falls back to scanning the repository, which is exactly what
/// the index exists to avoid.
fn install_permissions(
    root: &Path,
    settings_dir: &Path,
    server_name: &str,
    report: &mut InstallReport,
) -> Result<()> {
    let target = settings_dir.join("settings.json");
    let rule = format!("mcp__{server_name}__*");
    let changed = update_json_file(&target, |document| {
        let Some(map) = document.as_object_mut() else {
            return false;
        };
        let permissions = map
            .entry("permissions".to_string())
            .or_insert_with(|| json!({}));
        let Some(permissions) = permissions.as_object_mut() else {
            return false;
        };
        let allow = permissions
            .entry("allow".to_string())
            .or_insert_with(|| json!([]));
        let Some(list) = allow.as_array_mut() else {
            return false;
        };
        if list
            .iter()
            .any(|value| value.as_str() == Some(rule.as_str()))
        {
            return false;
        }
        list.push(json!(rule));
        true
    })?;
    if !changed {
        return Ok(());
    }
    backup_file(root, &target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: format!("permissao de uso das ferramentas MCP ({rule}) concedida"),
    });
    Ok(())
}

/// Removes the permission granted to the server.
fn remove_permissions(
    root: &Path,
    settings_dir: &Path,
    server_name: &str,
    report: &mut InstallReport,
) -> Result<()> {
    let target = settings_dir.join("settings.json");
    if !target.exists() {
        return Ok(());
    }
    let rule = format!("mcp__{server_name}__*");
    let changed = update_json_file(&target, |document| {
        let Some(map) = document.as_object_mut() else {
            return false;
        };
        let Some(permissions) = map.get_mut("permissions").and_then(|p| p.as_object_mut()) else {
            return false;
        };
        let Some(list) = permissions.get_mut("allow").and_then(|a| a.as_array_mut()) else {
            return false;
        };
        let before = list.len();
        list.retain(|value| value.as_str() != Some(rule.as_str()));
        before != list.len()
    })?;
    if !changed {
        return Ok(());
    }
    backup_file(root, &target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "permissao de uso das ferramentas MCP removida".into(),
    });
    Ok(())
}

/// Files where the rules and the hooks live for a given scope.
fn scope_paths(root: &Path, scope: InstallScope) -> (PathBuf, PathBuf) {
    match scope {
        InstallScope::User => (
            devzymer_core::paths::user_home()
                .join(".claude")
                .join("CLAUDE.md"),
            devzymer_core::paths::user_home().join(".claude"),
        ),
        InstallScope::Project | InstallScope::Local => {
            (root.join("CLAUDE.md"), root.join(".claude"))
        }
    }
}

fn remove_mcp_entry(
    root: &Path,
    target: &Path,
    path: &[&str],
    report: &mut InstallReport,
) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let changed = update_json_file(target, |document| {
        remove_json_path(document, path);
        true
    })?;
    if !changed {
        return Ok(());
    }
    backup_file(root, target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "entrada do servidor MCP removida".into(),
    });
    Ok(())
}

fn remove_rules(rules_path: &Path, root: &Path, report: &mut InstallReport) -> Result<()> {
    let target = rules_path.to_path_buf();
    if !target.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(&target)?;
    if !existing.contains(RULES_START) || !existing.contains(RULES_END) {
        return Ok(());
    }
    let start = existing.find(RULES_START).unwrap_or(0);
    let end = existing
        .find(RULES_END)
        .map(|e| e + RULES_END.len())
        .unwrap_or(existing.len());
    let mut updated = existing[..start].to_string();
    updated.push_str(existing[end..].trim_start_matches('\n'));
    backup_file(root, &target, report)?;
    write_atomic(&target, updated.as_bytes())?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "regras do devzymer removidas".into(),
    });
    Ok(())
}

fn remove_hooks(settings_dir: &Path, root: &Path, report: &mut InstallReport) -> Result<()> {
    let target = settings_dir.join("settings.json");
    if !target.exists() {
        return Ok(());
    }
    let changed = update_json_file(&target, |document| {
        let Some(hooks) = document.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
            return false;
        };
        for event in ["PostToolUse", "PreToolUse"] {
            let Some(list) = hooks.get_mut(event).and_then(|l| l.as_array_mut()) else {
                continue;
            };
            list.retain(|group| !group_is_devzymer(group));
        }
        true
    })?;
    if !changed {
        return Ok(());
    }
    backup_file(root, &target, report)?;
    report.changes.push(Change {
        file: target.display().to_string(),
        action: "updated".into(),
        detail: "hooks do devzymer removidos".into(),
    });
    Ok(())
}

pub fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => serde_json::from_str(&text).or_else(|_| Ok(json!({}))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(devzymer_core::Error::io(path, e)),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(devzymer_core::Error::Json)?;
    write_atomic(path, format!("{text}\n").as_bytes())
}

fn set_json_path(document: &mut Value, path: &[&str], value: Value) {
    if path.is_empty() {
        *document = value;
        return;
    }
    if !document.is_object() {
        *document = json!({});
    }
    let mut current = document;
    for (index, key) in path.iter().enumerate() {
        let map = current.as_object_mut().expect("document is an object");
        if index == path.len() - 1 {
            map.insert((*key).to_string(), value);
            return;
        }
        let entry = map.entry((*key).to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        current = entry;
    }
}

fn remove_json_path(document: &mut Value, path: &[&str]) {
    if path.is_empty() {
        return;
    }
    let mut current = document;
    for (index, key) in path.iter().enumerate() {
        let Some(map) = current.as_object_mut() else {
            return;
        };
        if index == path.len() - 1 {
            map.remove(*key);
            return;
        }
        let Some(next) = map.get_mut(*key) else {
            return;
        };
        current = next;
    }
}

pub(crate) fn backup_file(root: &Path, target: &Path, report: &mut InstallReport) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let stamp = devzymer_core::util::now_millis();
    let dir = devzymer_core::paths::backup_dir(root).join(stamp.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| devzymer_core::Error::io(&dir, e))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "backup".to_string());
    let destination = dir.join(&name);
    std::fs::copy(target, &destination).map_err(|e| devzymer_core::Error::io(target, e))?;
    report.backups.push(destination.display().to_string());
    Ok(())
}

/// Agent rules installed into the project instructions file.
///
/// The rules claim priority over any other code search tool of the project,
/// because the index answers with the exact path, symbol and line range while
/// the other tools scan the repository.
pub fn rules_block() -> String {
    format!(
        "{RULES_START}
## devzymer

Este repositório está indexado localmente pelo devzymer. Estas regras têm prioridade
sobre qualquer outra ferramenta de busca de código do projeto.

1. Antes de usar `grep`, `glob`, `Read` de arquivo inteiro, `codegraph_explore` ou
   qualquer outra ferramenta de busca, use `context_resolve`. Ele devolve o caminho,
   o símbolo e a faixa de linhas exata, sem varrer o repositório.
2. Depois de `context_resolve`, leia apenas a faixa devolvida, com `context_read` por
   símbolo ou por linhas. Não leia o arquivo inteiro.
3. Use `context_impact` antes de alterar código compartilhado, para ver quem depende
   dele.
4. Use `context_patch` para alterações mínimas, com verificação de revisão. A edição é
   rejeitada se o arquivo mudou desde a leitura.
5. Depois de criar, renomear ou apagar arquivos, chame `context_sync` para atualizar
   o índice.
6. Uma resposta com `ambiguidade:` significa que o nome tem mais de uma definição:
   use o nome qualificado (`Classe.metodo`) para desambiguar.
7. Evite releitura do mesmo trecho e explicações repetidas.
8. Use `context_decide` apenas quando as regras determinísticas não forem suficientes.
9. Preserve a correção do código: economia de tokens nunca justifica alterar o
   comportamento.
10. Responda de forma concisa em português do Brasil, e explique bloqueios de forma
    direta e precisa.
{RULES_END}"
    )
}

/// The agent rules used by the agents that read `AGENTS.md`.
///
/// The wording avoids naming tools that only exist in one host, because the
/// same file can be read by more than one agent.
pub fn rules_block_generic() -> String {
    format!(
        "{RULES_START}
## devzymer

Este repositório está indexado localmente pelo devzymer. Estas regras têm prioridade
sobre qualquer outra ferramenta de busca de código do projeto.

1. Antes de usar qualquer ferramenta de busca de código ou de leitura de arquivo
   inteiro, use a ferramenta `context_resolve` do servidor MCP `devzymer`. No
   OpenCode ela aparece como `devzymer_context_resolve`; em qualquer host, é a
   ferramenta do servidor devzymer que devolve o caminho, o símbolo e a faixa de
   linhas exata, sem varrer o repositório.
2. Depois de `context_resolve`, leia apenas a faixa devolvida, com `context_read` por
   símbolo ou por linhas. Não leia o arquivo inteiro.
3. Use `context_impact` antes de alterar código compartilhado, para ver quem depende
   dele.
4. Use `context_patch` para alterações mínimas, com verificação de revisão. A edição é
   rejeitada se o arquivo mudou desde a leitura, ou se o intervalo não estiver dentro
   do trecho lido. Sempre envie o `expected_sha` devolvido pela leitura.
5. Depois de criar, renomear ou apagar arquivos, chame `context_sync` para atualizar
   o índice.
6. Uma resposta com `ambiguidade:` significa que o nome tem mais de uma definição:
   use o nome qualificado (`Classe.metodo`) para desambiguar.
7. Evite releitura do mesmo trecho e explicações repetidas.
8. Use `context_decide` apenas quando as regras determinísticas não forem suficientes.
9. Preserve a correção do código: economia de tokens nunca justifica alterar o
   comportamento.
10. Responda de forma concisa em português do Brasil, e explique bloqueios de forma
    direta e precisa.
{RULES_END}"
    )
}

/// Merges two JSON documents, used by the migration flow.
pub fn merge_documents(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            let mut result = Map::new();
            for (k, v) in b {
                result.insert(k.clone(), v.clone());
            }
            for (k, v) in o {
                match (result.get(k), v) {
                    (Some(existing), Value::Object(_)) if existing.is_object() => {
                        let merged = merge_documents(existing, v);
                        result.insert(k.clone(), merged);
                    }
                    _ => {
                        result.insert(k.clone(), v.clone());
                    }
                }
            }
            Value::Object(result)
        }
        (_, overlay) => overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-install-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_local_index_is_kept_out_of_the_version_control() {
        let root = project("gitignore");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "node_modules\n").unwrap();

        let mut report = InstallReport::default();
        ensure_gitignore(&root, &mut report).unwrap();

        let content = std::fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(content.contains("node_modules"));
        assert!(content.contains(".devzymer/"));

        let before = content.clone();
        let mut second = InstallReport::default();
        ensure_gitignore(&root, &mut second).unwrap();
        assert_eq!(
            before,
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "the entry must not be duplicated"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_outside_a_repository_keeps_its_own_layout() {
        let root = project("no-git");
        let mut report = InstallReport::default();
        ensure_gitignore(&root, &mut report).unwrap();
        assert!(!root.join(".gitignore").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_existing_entry_is_not_duplicated() {
        assert!(gitignore_covers(".devzymer/\n", ".devzymer"));
        assert!(gitignore_covers("/.devzymer\n", ".devzymer"));
        assert!(!gitignore_covers("# .devzymer\n", ".devzymer"));
        assert!(!gitignore_covers("node_modules\n", ".devzymer"));
    }

    #[test]
    fn installation_adds_the_server_without_touching_the_others() {
        let root = project("add");
        std::fs::write(
            root.join(".mcp.json"),
            r#"{"mcpServers":{"outro":{"type":"stdio","command":"x"}}}"#,
        )
        .unwrap();

        let config = Config::default();
        let report = install(&root, &config, InstallScope::Project, None).unwrap();
        assert!(!report.changes.is_empty());

        let document = read_json(&root.join(".mcp.json")).unwrap();
        let servers = document["mcpServers"].as_object().unwrap();
        assert!(servers.contains_key("devzymer"));
        assert!(
            servers.contains_key("outro"),
            "other servers must be preserved"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installation_is_idempotent() {
        let root = project("idempotent");
        let config = Config::default();
        install(&root, &config, InstallScope::Project, None).unwrap();
        let first = std::fs::read_to_string(root.join(".mcp.json")).unwrap();

        let report = install(&root, &config, InstallScope::Project, None).unwrap();
        let second = std::fs::read_to_string(root.join(".mcp.json")).unwrap();
        assert_eq!(first, second, "a second install must not change the file");
        assert!(report.changes.iter().any(|c| c.action == "unchanged"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rules_are_installed_and_removed_without_losing_other_content() {
        let root = project("rules");
        std::fs::write(root.join("CLAUDE.md"), "# Projeto\n\nregra existente\n").unwrap();
        let config = Config::default();

        install(&root, &config, InstallScope::Project, None).unwrap();
        let content = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(content.contains("regra existente"));
        assert!(content.contains(RULES_START));

        uninstall(&root, &config.mcp.server_name, InstallScope::Project).unwrap();
        let content = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(content.contains("regra existente"));
        assert!(!content.contains(RULES_START));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn uninstall_keeps_foreign_hooks() {
        let root = project("hooks");
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(
            root.join(".claude").join("settings.json"),
            r#"{"hooks":{"PostToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"meu-script.sh"}]}]}}"#,
        )
        .unwrap();
        let config = Config::default();

        install(&root, &config, InstallScope::Project, None).unwrap();
        let document = read_json(&root.join(".claude").join("settings.json")).unwrap();
        let groups = document["hooks"]["PostToolUse"].as_array().unwrap();
        assert!(groups.len() >= 2, "the foreign hook must be preserved");

        uninstall(&root, &config.mcp.server_name, InstallScope::Project).unwrap();
        let document = read_json(&root.join(".claude").join("settings.json")).unwrap();
        let groups = document["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["hooks"][0]["command"], "meu-script.sh");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn json_paths_are_set_and_removed() {
        let mut document = json!({});
        set_json_path(&mut document, &["a", "b", "c"], json!(1));
        assert_eq!(document["a"]["b"]["c"], 1);
        remove_json_path(&mut document, &["a", "b", "c"]);
        assert!(document["a"]["b"].get("c").is_none());
    }
}
