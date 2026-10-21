use std::io::Read;

use anyhow::Result;
use serde_json::{json, Value};

use crate::HookKind;

const COMPRESS_THRESHOLD: usize = 3000;
const HEAD_LINES: usize = 15;
const TAIL_LINES: usize = 25;
const MAX_ERROR_LINES: usize = 30;

pub fn run(kind: HookKind) -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    let input: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!({}));
    match kind {
        HookKind::PostToolUse => post_tool_use(&input),
        HookKind::GuardTestFiles => guard_test_files(&input),
        HookKind::GuardSearch => guard_search(&input),
    }
}

/// Compresses an oversized tool result while preserving the evidence needed to
/// debug it. A failed operation is never presented as a successful one.
fn post_tool_use(input: &Value) -> Result<()> {
    let tool_name = input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if tool_name != "Bash" {
        return Ok(());
    }
    let Some(response) = input.get("tool_response").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    let stdout = response
        .get("stdout")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let stderr = response
        .get("stderr")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if stdout.len() < COMPRESS_THRESHOLD && stderr.len() < COMPRESS_THRESHOLD {
        return Ok(());
    }

    let mut updated = response.clone();
    updated.insert("stdout".to_string(), json!(compress(stdout)));
    if stderr.is_empty() {
        updated.insert("stderr".to_string(), json!(""));
    } else {
        updated.insert(
            "stderr".to_string(),
            json!(format!(
                "[devzymer] stderr presente: a operação pode ter falhado.\n{}",
                compress(stderr)
            )),
        );
    }

    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": Value::Object(updated)
        }
    });
    println!("{output}");
    Ok(())
}

/// Deterministic, schema aware compression of a command output.
fn compress(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= HEAD_LINES + TAIL_LINES {
        return text.to_string();
    }

    let mut out: Vec<String> = Vec::with_capacity(HEAD_LINES + TAIL_LINES + 40);
    for line in lines.iter().take(HEAD_LINES) {
        out.push((*line).to_string());
    }

    let error_lines: Vec<&str> = lines
        .iter()
        .skip(HEAD_LINES)
        .take(lines.len().saturating_sub(HEAD_LINES + TAIL_LINES))
        .filter(|line| is_error_line(line))
        .copied()
        .take(MAX_ERROR_LINES)
        .collect();

    let omitted = lines.len() - HEAD_LINES - TAIL_LINES - error_lines.len();
    out.push(format!(
        "[devzymer] {} linha(s) omitida(s) pela compressão de saída.",
        omitted
    ));

    if !error_lines.is_empty() {
        out.push("[devzymer] linhas relevantes preservadas:".to_string());
        for line in error_lines {
            out.push(line.to_string());
        }
    }

    out.push("[devzymer] últimas linhas:".to_string());
    for line in lines.iter().rev().take(TAIL_LINES).rev() {
        out.push((*line).to_string());
    }

    out.join("\n")
}

fn is_error_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    const MARKERS: [&str; 14] = [
        "error",
        "failed",
        "failure",
        "exception",
        "traceback",
        "panic:",
        "fatal",
        "denied",
        "cannot ",
        "unable to",
        "warning:",
        "npm err",
        "assertion",
        "segmentation fault",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Blocks creation of dedicated test files until the user approves them.
fn guard_test_files(input: &Value) -> Result<()> {
    let tool_name = input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_input = input
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let candidates: Vec<String> = match tool_name {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => tool_input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        "Bash" => tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .map(paths_from_command)
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let offenders: Vec<String> = candidates
        .into_iter()
        .filter(|candidate| is_test_path(candidate))
        .collect();
    if offenders.is_empty() {
        return Ok(());
    }

    let root = devzymer_core::paths::claude_project_dir()
        .unwrap_or_else(devzymer_core::paths::current_dir);
    let config = devzymer_core::config::Config::load(&root).unwrap_or_default();
    if config.tests.allow_files {
        return Ok(());
    }
    if !config.tests.approval_marker.is_empty() && root.join(&config.tests.approval_marker).exists()
    {
        return Ok(());
    }

    let reason = format!(
        "A criação de arquivos de teste dedicados exige aprovação explícita do usuário.\n\
         Arquivos bloqueados: {}\n\
         Explique ao usuário por que o arquivo de teste é necessário e o custo de gerá-lo, \
         e aguarde a aprovação. Para liberar de forma permanente, execute \
         `devzymer config set tests.allow_files true` ou crie o marcador `{}`.",
        offenders.join(", "),
        config.tests.approval_marker
    );

    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason
        }
    });
    println!("{output}");
    Ok(())
}

/// Extracts the file paths a shell command would create.
fn paths_from_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut expects_path = false;
    for raw in command.split_whitespace() {
        let token = raw.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '(' | ')' | ';'));
        if token.is_empty() {
            continue;
        }
        if let Some(index) = token.find('>') {
            let target = token[index + 1..].trim_matches(|c| matches!(c, '"' | '\'' | '`'));
            if !target.is_empty() {
                out.push(target.to_string());
            }
            continue;
        }
        if expects_path {
            if !token.starts_with('-') {
                out.push(token.to_string());
            }
            expects_path = false;
            continue;
        }
        let base = token.trim_end_matches(':');
        if matches!(base, "touch" | "tee" | "cp" | "mv" | "install" | "mkdir") {
            expects_path = true;
        }
    }
    out
}

/// True for the conventional dedicated test file locations.
fn is_test_path(path: &str) -> bool {
    devzymer_core::util::is_test_path(path)
}

/// Redirects the code search of the agent to the local index.
///
/// The agent decides on its own whether to use the index, and with many tools
/// available it often chooses a full repository scan. This guard denies the
/// scan only when the index can answer it, and points to the tool that
/// answers it.
fn guard_search(input: &Value) -> Result<()> {
    let tool_name = input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let tool_input = input
        .get("tool_input")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let cwd = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(devzymer_core::paths::current_dir);
    let root = devzymer_core::paths::find_workspace_root(&cwd);
    let config = devzymer_core::config::Config::load(&root).unwrap_or_default();
    if !config.context.guard_search {
        return Ok(());
    }
    let db = devzymer_core::paths::db_path(&root);
    if !db.exists() {
        return Ok(());
    }
    let Ok(store) = devzymer_core::store::Store::open(&db) else {
        return Ok(());
    };

    let reason = match tool_name {
        "Grep" => {
            let pattern = tool_input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            match grep_target(&store, pattern) {
                Some(term) => format!(
                    "Este repositório está indexado localmente. Use \
                     `mcp__devzymer__context_resolve` com a consulta \
                     \"{term}\": ele devolve o caminho, o símbolo e a faixa de \
                     linhas exata, sem varrer o repositório. Depois use \
                     `mcp__devzymer__context_read` para ler apenas essa faixa. \
                     Se a busca for por texto que não é código, use o Grep \
                     novamente."
                ),
                None => return Ok(()),
            }
        }
        "Read" => {
            let path = tool_input
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let relative = relative_path(&root, path);
            let known = store.file_by_path(&relative).ok().flatten().is_some();
            if !known {
                return Ok(());
            }
            format!(
                "Este arquivo está indexado localmente. Use \
                 `mcp__devzymer__context_read` com path=\"{relative}\": devolve o \
                 mesmo conteúdo com numeração de linhas, e aceita \
                 start_line/end_line ou symbol para ler apenas o trecho \
                 necessário."
            )
        }
        "Glob" => {
            let pattern = tool_input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if pattern.contains('*') || pattern.contains('?') {
                return Ok(());
            }
            let stem = devzymer_core::store::stem_of(pattern);
            let files = store.files_by_stem(&stem).unwrap_or_default();
            let name = pattern
                .rsplit_once('.')
                .map(|(base, _)| base)
                .unwrap_or(pattern);
            match files.first() {
                Some(file) => format!(
                    "Use `mcp__devzymer__context_resolve` com a consulta \"{}\": o índice local devolve o caminho e a \
                     faixa de linhas de {}.",
                    name, file.path
                ),
                None => return Ok(()),
            }
        }
        _ => return Ok(()),
    };

    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason
        }
    });
    println!("{output}");
    Ok(())
}

/// The identifier of a search pattern that the index can resolve, if any.
fn grep_target(store: &devzymer_core::store::Store, pattern: &str) -> Option<String> {
    for raw in pattern.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '$') {
        let candidate = raw.trim_matches('.');
        if candidate.len() < 4 {
            continue;
        }
        if !candidate
            .chars()
            .skip(1)
            .any(|c| c.is_uppercase() || c == '_')
        {
            continue;
        }
        if !store
            .symbols_by_name(candidate)
            .unwrap_or_default()
            .is_empty()
        {
            return Some(candidate.to_string());
        }
        if !store
            .files_by_stem(&devzymer_core::store::stem_of(candidate))
            .unwrap_or_default()
            .is_empty()
        {
            let name = candidate
                .rsplit_once('.')
                .map(|(base, _)| base)
                .unwrap_or(candidate);
            return Some(name.to_string());
        }
    }
    None
}

/// The path of a file relative to the repository root.
fn relative_path(root: &std::path::Path, path: &str) -> String {
    let candidate = std::path::Path::new(path);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    match absolute.strip_prefix(root) {
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => path.replace('\\', "/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(path: &str, name: &str) -> devzymer_core::store::Store {
        let store = devzymer_core::store::Store::open_in_memory().unwrap();
        let record = devzymer_core::store::FileRecord {
            id: 0,
            path: path.to_string(),
            language: devzymer_core::model::Language::TypeScript,
            size_bytes: 10,
            mtime_ms: 1,
            content_hash: "x".to_string(),
            symbol_count: 1,
            indexed_at_ms: 1,
        };
        let file_id = store.upsert_file(&record).unwrap();
        let symbol = devzymer_core::model::Symbol {
            id: devzymer_core::model::SymbolId::new(path, name),
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: devzymer_core::model::SymbolKind::Function,
            container: None,
            signature: None,
            doc: None,
            path: path.to_string(),
            language: devzymer_core::model::Language::TypeScript,
            span: devzymer_core::model::Span::new(0, 10, 1, 2),
            exported: true,
            is_async: false,
            body: None,
        };
        store.replace_symbols(file_id, &[symbol]).unwrap();
        store
    }

    #[test]
    fn an_identifier_of_the_index_is_redirected() {
        let store = store_with("src/hooks/useVoiceCall.ts", "useLeituraVoiceCall");
        assert_eq!(
            grep_target(&store, "useVoiceCall").as_deref(),
            Some("useVoiceCall")
        );
    }

    #[test]
    fn a_plain_word_is_left_to_the_grep_tool() {
        let store = store_with("src/hooks/useVoiceCall.ts", "useLeituraVoiceCall");
        assert_eq!(grep_target(&store, "TODO"), None);
        assert_eq!(grep_target(&store, "error"), None);
    }

    #[test]
    fn a_file_name_of_the_index_is_redirected() {
        let store = store_with("src/hooks/useVoiceCall.ts", "useLeituraVoiceCall");
        assert_eq!(
            grep_target(&store, "useVoiceCall.ts").as_deref(),
            Some("useVoiceCall")
        );
    }

    #[test]
    fn a_relative_path_is_taken_from_the_repository_root() {
        let root = std::path::Path::new("/repo");
        assert_eq!(relative_path(root, "/repo/src/lib.rs"), "src/lib.rs");
        assert_eq!(relative_path(root, "src/lib.rs"), "src/lib.rs");
    }
}
