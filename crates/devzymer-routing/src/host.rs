use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Host {
    Claude,
    Codex,
    OpenCode,
    #[default]
    Unknown,
}

impl Host {
    pub fn parse(value: &str) -> Option<Host> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Some(Host::Claude),
            "codex" => Some(Host::Codex),
            "opencode" => Some(Host::OpenCode),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Host::Claude => "claude",
            Host::Codex => "codex",
            Host::OpenCode => "opencode",
            Host::Unknown => "unknown",
        }
    }

    pub fn from_client_name(name: &str) -> Host {
        let lower = name.to_ascii_lowercase();
        if lower.contains("claude") {
            Host::Claude
        } else if lower.contains("codex") {
            Host::Codex
        } else if lower.contains("opencode") {
            Host::OpenCode
        } else {
            Host::Unknown
        }
    }

    pub fn from_hook_input(input: &Value) -> Host {
        if input.get("turn_id").is_some() {
            Host::Codex
        } else if input.get("hook_event_name").is_some() || input.get("session_id").is_some() {
            Host::Claude
        } else {
            Host::Unknown
        }
    }

    pub fn mcp_tool_prefix(self) -> &'static str {
        match self {
            Host::OpenCode => "devzymer_",
            _ => "mcp__devzymer__",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_host_is_recognized_from_its_flag() {
        assert_eq!(Host::parse("Claude"), Some(Host::Claude));
        assert_eq!(Host::parse("claude-code"), Some(Host::Claude));
        assert_eq!(Host::parse("codex"), Some(Host::Codex));
        assert_eq!(Host::parse("opencode"), Some(Host::OpenCode));
        assert_eq!(Host::parse("vim"), None);
    }

    #[test]
    fn a_host_is_recognized_from_the_mcp_client_name() {
        assert_eq!(Host::from_client_name("claude-code"), Host::Claude);
        assert_eq!(Host::from_client_name("codex-mcp-client"), Host::Codex);
        assert_eq!(Host::from_client_name("OpenCode"), Host::OpenCode);
        assert_eq!(Host::from_client_name("something"), Host::Unknown);
    }

    #[test]
    fn the_hook_payload_tells_codex_from_claude() {
        assert_eq!(
            Host::from_hook_input(&json!({"turn_id": "t", "session_id": "s"})),
            Host::Codex
        );
        assert_eq!(
            Host::from_hook_input(&json!({"hook_event_name": "PreToolUse"})),
            Host::Claude
        );
        assert_eq!(Host::from_hook_input(&json!({})), Host::Unknown);
    }
}
