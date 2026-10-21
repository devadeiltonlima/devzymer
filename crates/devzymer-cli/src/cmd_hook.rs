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
        HookKind::GuardSearch { host } => crate::routing_hook::guard(&input, host.as_deref()),
        HookKind::NoteMcp { host } => crate::routing_hook::note_mcp(&input, host.as_deref()),
        HookKind::NotePrompt { host } => crate::routing_hook::note_prompt(&input, host.as_deref()),
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
