use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
#[derive(Default)]
pub struct Config {
    pub index: IndexConfig,
    pub search: SearchConfig,
    pub context: ContextConfig,
    pub decisions: DecisionsConfig,
    pub mcp: McpConfig,
    pub privacy: PrivacyConfig,
    pub logging: LoggingConfig,
    pub tests: TestsConfig,
}

/// Controls the permission aware hook that guards dedicated test files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct TestsConfig {
    /// Allows creating test files without an explicit approval.
    pub allow_files: bool,
    /// Marker file that grants approval for the project.
    pub approval_marker: String,
}

impl Default for TestsConfig {
    fn default() -> Self {
        Self {
            allow_files: false,
            approval_marker: ".devzymer/tests.approved".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct IndexConfig {
    pub exclude: Vec<String>,
    pub max_file_size_bytes: u64,
    pub follow_symlinks: bool,
    pub respect_gitignore: bool,
    pub include_hidden: bool,
    pub languages: Vec<String>,
    pub max_workers: usize,
    pub memory_budget_mb: usize,
    /// Keeps the index synchronized while the MCP server runs.
    pub watching: bool,
    /// Time budget for an automatic synchronization, in milliseconds.
    pub sync_budget_ms: u64,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            exclude: [
                "node_modules",
                ".git",
                ".hg",
                ".svn",
                "dist",
                "build",
                "out",
                "target",
                "vendor",
                "coverage",
                ".next",
                ".nuxt",
                ".output",
                ".svelte-kit",
                ".turbo",
                ".parcel-cache",
                "__pycache__",
                ".venv",
                "venv",
                ".tox",
                ".mypy_cache",
                ".pytest_cache",
                ".gradle",
                ".idea",
                ".terraform",
                "Pods",
                "DerivedData",
                "bin",
                "obj",
                ".devzymer",
                ".codegraph",
                "graphify-out",
                ".worktrees",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            max_file_size_bytes: 2_000_000,
            follow_symlinks: false,
            respect_gitignore: true,
            include_hidden: false,
            languages: Vec::new(),
            max_workers: 0,
            memory_budget_mb: 512,
            watching: true,
            sync_budget_ms: 2000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SearchConfig {
    pub max_results: usize,
    pub default_token_budget: usize,
    pub snippet_max_lines: usize,
    pub max_fragment_bytes: usize,
    pub min_bm25_score: f64,
    /// Column weights of the FTS BM25 ranking, in the order the FTS table
    /// declares its columns: name, qualified_name, signature, doc. Identity
    /// columns outrank body text, so a word inside a doc comment cannot
    /// outweigh a word inside the symbol name.
    pub bm25_name_weight: f64,
    pub bm25_qualified_weight: f64,
    pub bm25_signature_weight: f64,
    pub bm25_doc_weight: f64,
    /// Weight of the source text column. The body is the only place where
    /// the behavior of a symbol is written down, so it competes with the
    /// name columns on equal terms.
    pub bm25_body_weight: f64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_results: 20,
            default_token_budget: 2000,
            snippet_max_lines: 40,
            max_fragment_bytes: 65_536,
            // The candidate limit already bounds the lexical rows, so the
            // floor only rejects the residual rows. A small corpus yields
            // small absolute bm25 values even for its best row.
            min_bm25_score: 0.05,
            bm25_name_weight: 1.0,
            bm25_qualified_weight: 1.0,
            bm25_signature_weight: 1.0,
            bm25_doc_weight: 1.0,
            bm25_body_weight: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ContextConfig {
    pub default_level: u8,
    pub max_total_bytes: usize,
    pub dedupe: bool,
    pub cache_enabled: bool,
    pub cache_ttl_secs: u64,
    /// Redirects the code search of the agent to the local index.
    pub guard_search: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            default_level: 2,
            max_total_bytes: 64_000,
            dedupe: true,
            cache_enabled: true,
            cache_ttl_secs: 900,
            guard_search: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct DecisionsConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub cache_ttl_secs: u64,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_secs: u64,
    pub max_state_chars: usize,
    pub min_confidence: f64,
}

impl Default for DecisionsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "typesafe".to_string(),
            model: "typesafe/jev-1.14".to_string(),
            base_url: "https://api.typesafe.ai".to_string(),
            api_key_env: "TYPESAFE_API_KEY".to_string(),
            // "typesafe" usa a API System One. "openrouter" usa o endpoint de
            // chat completions do OpenRouter com o mesmo modelo.
            timeout_secs: 10,
            max_retries: 1,
            cache_ttl_secs: 3600,
            circuit_breaker_threshold: 3,
            circuit_breaker_cooldown_secs: 60,
            max_state_chars: 6000,
            min_confidence: 0.55,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct McpConfig {
    pub server_name: String,
    pub protocol_version: String,
    pub install_scope: String,
    pub max_output_tokens: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            server_name: "devzymer".to_string(),
            protocol_version: "2025-06-18".to_string(),
            install_scope: "project".to_string(),
            max_output_tokens: 4000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PrivacyConfig {
    pub allow_remote_source: bool,
    pub redact_secrets: bool,
    pub exclude_secret_files: bool,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            allow_remote_source: false,
            redact_secrets: true,
            exclude_secret_files: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LoggingConfig {
    pub level: String,
    pub file: Option<PathBuf>,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            file: None,
        }
    }
}

impl Config {
    /// Loads the effective configuration for a project: defaults, then the
    /// user level file, then the project level file.
    pub fn load(root: &Path) -> Result<Config> {
        let mut value = toml::Value::Table(Default::default());
        merge_value(&mut value, &read_toml(&crate::paths::user_config_path())?);
        merge_value(
            &mut value,
            &read_toml(&crate::paths::project_config_path(root))?,
        );
        let config: Config = value
            .try_into()
            .map_err(|e: toml::de::Error| Error::Config(format!("invalid configuration: {e}")))?;
        Ok(config)
    }

    pub fn load_user_only() -> Result<Config> {
        let mut value = toml::Value::Table(Default::default());
        merge_value(&mut value, &read_toml(&crate::paths::user_config_path())?);
        let config: Config = value
            .try_into()
            .map_err(|e: toml::de::Error| Error::Config(format!("invalid configuration: {e}")))?;
        Ok(config)
    }

    pub fn save_project(&self, root: &Path) -> Result<PathBuf> {
        let path = crate::paths::project_config_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = toml::to_string_pretty(self)?;
        write_atomic(&path, text.as_bytes())?;
        Ok(path)
    }

    pub fn save_user(&self) -> Result<PathBuf> {
        let path = crate::paths::user_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let text = toml::to_string_pretty(self)?;
        write_atomic(&path, text.as_bytes())?;
        Ok(path)
    }

    pub fn effective_workers(&self) -> usize {
        if self.index.max_workers > 0 {
            return self.index.max_workers;
        }
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }

    /// Model name sent to the decision provider, with the vendor prefix
    /// removed when the provider expects a bare model identifier.
    pub fn decision_model(&self) -> String {
        match self.decisions.provider.as_str() {
            "typesafe" => self
                .decisions
                .model
                .rsplit('/')
                .next()
                .unwrap_or(&self.decisions.model)
                .to_string(),
            _ => self.decisions.model.clone(),
        }
    }
}

fn read_toml(path: &Path) -> Result<toml::Value> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(Default::default()))
        }
        Err(e) => Err(Error::io(path, e)),
    }
}

fn merge_value(target: &mut toml::Value, source: &toml::Value) {
    match (target, source) {
        (toml::Value::Table(t), toml::Value::Table(s)) => {
            for (k, v) in s {
                match t.get_mut(k) {
                    Some(existing) => merge_value(existing, v),
                    None => {
                        t.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (t, s) => *t = s.clone(),
    }
}

/// Writes bytes through a temporary file and renames it into place, so a
/// crash never leaves a half written configuration behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("devzymer")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(Error::io(path, e))
        }
    }
}
