use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpOutcome {
    Ok,
    NotFound,
    Ambiguous,
    BudgetExhausted,
    Error,
}

impl McpOutcome {
    pub fn is_failure(self) -> bool {
        !matches!(self, McpOutcome::Ok | McpOutcome::Ambiguous)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            McpOutcome::Ok => "ok",
            McpOutcome::NotFound => "not_found",
            McpOutcome::Ambiguous => "ambiguous",
            McpOutcome::BudgetExhausted => "budget_exhausted",
            McpOutcome::Error => "error",
        }
    }
}

pub fn extract_text(value: &Value) -> String {
    let mut out = String::new();
    collect_text(value, &mut out, 0);
    out
}

fn collect_text(value: &Value, out: &mut String, depth: u8) {
    if depth > 6 || out.len() > 4_000 {
        return;
    }
    match value {
        Value::String(text) => {
            out.push_str(text);
            out.push('\n');
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_text(item, out, depth + 1)),
        Value::Object(map) => {
            if let Some(text) = map.get("text") {
                collect_text(text, out, depth + 1);
            }
            if let Some(content) = map.get("content") {
                collect_text(content, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn is_error_flag(value: &Value) -> bool {
    value
        .get("isError")
        .or_else(|| value.get("is_error"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub fn classify_response(tool: &str, response: &Value) -> McpOutcome {
    let text = extract_text(response);
    classify_text(tool, &text, is_error_flag(response))
}

pub fn classify_text(tool: &str, text: &str, is_error: bool) -> McpOutcome {
    let head: String = text.trim_start().chars().take(240).collect();
    let lower = head.to_ascii_lowercase();
    if is_error || lower.starts_with("err ") || lower.starts_with("error") {
        return McpOutcome::Error;
    }
    let full = text.to_ascii_lowercase();
    let lookup = tool.ends_with("context_resolve") || tool.ends_with("context_impact");
    if lookup && (full.contains("not_found: true") || lower.starts_with("nao_encontrado")) {
        return McpOutcome::NotFound;
    }
    if full.contains("| truncado |") || full.contains("output budget exhausted") {
        return McpOutcome::BudgetExhausted;
    }
    if lookup && full.contains("ambiguidade:") {
        return McpOutcome::Ambiguous;
    }
    McpOutcome::Ok
}

pub fn is_devzymer_tool(name: &str) -> bool {
    name.starts_with("mcp__devzymer__")
        || name.starts_with("devzymer_context_")
        || name.starts_with("context_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_zero_result_lookup_is_not_found_and_a_normal_one_is_ok() {
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_resolve",
                "not_found: true\nnao_encontrado: x",
                false
            ),
            McpOutcome::NotFound
        );
        assert_eq!(
            classify_text("mcp__devzymer__context_resolve", "nao_encontrado: x", false),
            McpOutcome::NotFound
        );
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_resolve",
                "arquivo: a.rs | 3 simbolo(s)\na.rs:1-2 | f | function | confirmed",
                false
            ),
            McpOutcome::Ok
        );
    }

    #[test]
    fn an_error_response_is_an_error_whatever_the_tool() {
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_read",
                "ERR PATH_NOT_FOUND: x",
                false
            ),
            McpOutcome::Error
        );
        assert_eq!(
            classify_text("mcp__devzymer__context_status", "anything", true),
            McpOutcome::Error
        );
    }

    #[test]
    fn ambiguity_and_budget_are_reported_for_lookups_only() {
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_resolve",
                "ambiguidade: dois candidatos",
                false
            ),
            McpOutcome::Ambiguous
        );
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_read",
                "ambiguidade: texto do codigo",
                false
            ),
            McpOutcome::Ok
        );
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_read",
                "a.rs:1-40 sha=ab bytes=900 tokens~200 | truncado | continuacao a partir da linha 41\ncodigo",
                false
            ),
            McpOutcome::BudgetExhausted
        );
        assert_eq!(
            classify_text(
                "mcp__devzymer__context_read",
                "a.rs:1-9 sha=ab bytes=90 tokens~20\ncodigo",
                false
            ),
            McpOutcome::Ok
        );
    }

    #[test]
    fn the_text_is_extracted_from_the_host_response_shapes() {
        let blocks = json!([{ "type": "text", "text": "nao_encontrado: abc" }]);
        assert_eq!(
            classify_response("mcp__devzymer__context_resolve", &blocks),
            McpOutcome::NotFound
        );
        let wrapped = json!({ "content": [{ "type": "text", "text": "ok" }], "isError": true });
        assert_eq!(
            classify_response("mcp__devzymer__context_resolve", &wrapped),
            McpOutcome::Error
        );
        let plain = json!("a.rs:1-2 | f | function | confirmed");
        assert_eq!(
            classify_response("mcp__devzymer__context_resolve", &plain),
            McpOutcome::Ok
        );
    }

    #[test]
    fn only_devzymer_tools_are_recognized() {
        assert!(is_devzymer_tool("mcp__devzymer__context_resolve"));
        assert!(is_devzymer_tool("devzymer_context_read"));
        assert!(!is_devzymer_tool("mcp__other__search"));
        assert!(!is_devzymer_tool("Grep"));
    }
}
