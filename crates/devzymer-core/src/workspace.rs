use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::util::slash;

#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    pub root: String,
    pub is_worktree: bool,
    pub common_dir: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ClaudeDetection {
    pub cli_installed: bool,
    pub user_config: Option<PathBuf>,
    pub project_mcp: Option<PathBuf>,
    pub project_settings: Option<PathBuf>,
    pub project_rules: Option<PathBuf>,
    pub user_rules: Option<PathBuf>,
    pub installed_servers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CodeGraphDetection {
    pub present: bool,
    pub index_dir: Option<PathBuf>,
    pub mcp_entries: Vec<String>,
    pub executable: Option<PathBuf>,
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Workspace {
    pub root: PathBuf,
    pub git: Option<GitInfo>,
    pub nested_repos: Vec<String>,
    pub manifests: Vec<String>,
    pub package_managers: Vec<String>,
    pub languages: Vec<String>,
    pub claude: ClaudeDetection,
    pub codegraph: CodeGraphDetection,
    pub trust: TrustLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    #[default]
    Unknown,
    Trusted,
    Untrusted,
}

impl Workspace {
    pub fn detect(root: &Path) -> Workspace {
        let mut ws = Workspace {
            root: root.to_path_buf(),
            ..Default::default()
        };
        ws.git = detect_git(root);
        ws.nested_repos = detect_nested_repos(root);
        ws.manifests = detect_manifests(root);
        ws.package_managers = detect_package_managers(root);
        ws.languages = detect_languages(root);
        ws.claude = detect_claude(root);
        ws.codegraph = detect_codegraph(root);
        ws.trust = detect_trust(root);
        ws
    }
}

fn detect_git(root: &Path) -> Option<GitInfo> {
    let git_path = root.join(".git");
    if !git_path.exists() {
        return None;
    }
    let mut info = GitInfo {
        root: slash(root),
        is_worktree: false,
        common_dir: None,
        branch: None,
    };
    if git_path.is_dir() {
        let head = root.join(".git").join("HEAD");
        if let Ok(text) = std::fs::read_to_string(&head) {
            let trimmed = text.trim();
            if let Some(rest) = trimmed.strip_prefix("ref: ") {
                info.branch = Some(rest.replace("refs/heads/", ""));
            }
        }
        if let Ok(text) = std::fs::read_to_string(root.join(".git").join("commondir")) {
            info.common_dir = Some(text.trim().to_string());
        }
    } else if let Ok(text) = std::fs::read_to_string(&git_path) {
        info.is_worktree = true;
        if let Some(rest) = text.trim().strip_prefix("gitdir: ") {
            let gitdir = PathBuf::from(rest);
            if let Some(parent) = gitdir.parent() {
                info.common_dir = Some(slash(parent));
            }
        }
    }
    Some(info)
}

fn detect_nested_repos(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 5000usize;
    while let Some(dir) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | ".git" | "vendor" | "__pycache__"
            ) {
                continue;
            }
            if path.join(".git").exists() {
                if path != root {
                    found.push(slash(&path));
                }
                continue;
            }
            stack.push(path);
        }
    }
    found.sort();
    found
}

const MANIFESTS: &[(&str, &str)] = &[
    ("package.json", "npm"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("bun.lockb", "bun"),
    ("Cargo.toml", "cargo"),
    ("pyproject.toml", "pip"),
    ("requirements.txt", "pip"),
    ("Pipfile", "pipenv"),
    ("poetry.lock", "poetry"),
    ("go.mod", "go"),
    ("pom.xml", "maven"),
    ("build.gradle", "gradle"),
    ("build.gradle.kts", "gradle"),
    ("composer.json", "composer"),
    ("Gemfile", "bundler"),
    ("Package.swift", "swiftpm"),
    ("mix.exs", "mix"),
    ("pubspec.yaml", "pub"),
    ("deno.json", "deno"),
    ("deno.jsonc", "deno"),
];

fn detect_manifests(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (file, _) in MANIFESTS {
        if root.join(file).exists() {
            out.push((*file).to_string());
        }
    }
    out
}

fn detect_package_managers(_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, manager) in MANIFESTS {
        if !out.contains(&manager.to_string()) {
            out.push(manager.to_string());
        }
    }
    out
}

fn detect_languages(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 3000usize;
    while let Some(dir) = stack.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if matches!(
                    name.as_str(),
                    "node_modules"
                        | "target"
                        | "dist"
                        | "build"
                        | ".git"
                        | "vendor"
                        | "__pycache__"
                ) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let lang = crate::model::Language::from_path(&path);
            if lang != crate::model::Language::Unknown {
                let name = lang.display().to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out.sort();
    out
}

fn detect_claude(root: &Path) -> ClaudeDetection {
    let mut det = ClaudeDetection {
        cli_installed: which("claude").is_some(),
        ..Default::default()
    };
    let user_config = crate::paths::user_home().join(".claude.json");
    if user_config.exists() {
        det.user_config = Some(user_config);
    }
    let project_mcp = root.join(".mcp.json");
    if project_mcp.exists() {
        det.project_mcp = Some(project_mcp.clone());
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(
            &std::fs::read_to_string(&project_mcp).unwrap_or_default(),
        ) {
            if let Some(map) = value.get("mcpServers").and_then(|v| v.as_object()) {
                det.installed_servers = map.keys().cloned().collect();
            }
        }
    }
    let project_settings = root.join(".claude").join("settings.json");
    if project_settings.exists() {
        det.project_settings = Some(project_settings);
    }
    let project_rules = root.join("CLAUDE.md");
    if project_rules.exists() {
        det.project_rules = Some(project_rules);
    }
    let user_rules = crate::paths::user_home().join(".claude").join("CLAUDE.md");
    if user_rules.exists() {
        det.user_rules = Some(user_rules);
    }
    det.installed_servers.sort();
    det
}

fn detect_codegraph(root: &Path) -> CodeGraphDetection {
    let mut det = CodeGraphDetection::default();
    let index_dir = root.join(".codegraph");
    if index_dir.exists() {
        det.index_dir = Some(index_dir);
        det.present = true;
    }
    for candidate in [
        root.join(".mcp.json"),
        crate::paths::user_home().join(".claude.json"),
        root.join(".claude").join("settings.json"),
    ] {
        if !candidate.exists() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        if text.to_ascii_lowercase().contains("codegraph") {
            det.present = true;
            det.mcp_entries.push(candidate.display().to_string());
        }
    }
    if let Some(exe) = which("codegraph") {
        det.present = true;
        det.executable = Some(exe);
    }
    det.mcp_entries.sort();
    det
}

fn detect_trust(root: &Path) -> TrustLevel {
    if let Ok(text) = std::fs::read_to_string(root.join(".claude").join("settings.json")) {
        if text.contains("enableAllProjectMcpServers") {
            return TrustLevel::Trusted;
        }
    }
    if root.join(".git").exists() {
        TrustLevel::Trusted
    } else {
        TrustLevel::Untrusted
    }
}

/// Locates an executable on PATH using the platform conventions.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
