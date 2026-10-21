use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::claude::read_json;

#[derive(Debug, Clone, Default, Serialize)]
pub struct CodeGraphReport {
    pub present: bool,
    pub index_dir: Option<String>,
    pub mcp_entries: Vec<String>,
    pub mcp_server_names: Vec<String>,
    pub executable: Option<String>,
    pub scope: Vec<String>,
    pub notes: Vec<String>,
}

/// Detects an existing CodeGraph installation without modifying anything.
pub fn detect_codegraph(root: &Path) -> CodeGraphReport {
    let mut report = CodeGraphReport::default();

    let index_dir = root.join(".codegraph");
    if index_dir.exists() {
        report.present = true;
        report.index_dir = Some(index_dir.display().to_string());
        report.scope.push("project".into());
    }

    let candidates = [
        root.join(".mcp.json"),
        devzymer_core::paths::user_home().join(".claude.json"),
        root.join(".claude").join("settings.json"),
        devzymer_core::paths::user_home()
            .join(".claude")
            .join("settings.json"),
    ];

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let document = match read_json(&candidate) {
            Ok(doc) => doc,
            Err(_) => continue,
        };
        let text = document.to_string().to_ascii_lowercase();
        if !text.contains("codegraph") {
            continue;
        }
        report.present = true;
        report.mcp_entries.push(candidate.display().to_string());
        report.scope.push(scope_of(&candidate));
        for name in server_names(&document) {
            if name.to_ascii_lowercase().contains("codegraph") {
                report.mcp_server_names.push(name);
            }
        }
    }

    if let Some(exe) = devzymer_core::workspace::which("codegraph") {
        report.present = true;
        report.executable = Some(exe.display().to_string());
    }

    report.mcp_entries.sort();
    report.mcp_entries.dedup();
    report.mcp_server_names.sort();
    report.mcp_server_names.dedup();
    report.scope.sort();
    report.scope.dedup();
    report
}

fn scope_of(path: &Path) -> String {
    let home = devzymer_core::paths::user_home();
    if path.starts_with(&home) {
        "user".to_string()
    } else {
        "project".to_string()
    }
}

fn server_names(document: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(map) = document.get("mcpServers").and_then(|v| v.as_object()) {
        out.extend(map.keys().cloned());
    }
    if let Some(projects) = document.get("projects").and_then(|v| v.as_object()) {
        for project in projects.values() {
            if let Some(map) = project.get("mcpServers").and_then(|v| v.as_object()) {
                out.extend(map.keys().cloned());
            }
        }
    }
    out
}

/// Removes a CodeGraph MCP registration without removing the tool itself.
pub fn disable_mcp_entry(target: &Path, server_name: &str) -> Result<bool, String> {
    if !target.exists() {
        return Ok(false);
    }
    let mut document = read_json(target).map_err(|e| e.to_string())?;
    let mut changed = false;

    if let Some(map) = document
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    {
        if map.remove(server_name).is_some() {
            changed = true;
        }
    }
    if let Some(projects) = document.get_mut("projects").and_then(|v| v.as_object_mut()) {
        for project in projects.values_mut() {
            if let Some(map) = project
                .get_mut("mcpServers")
                .and_then(|v| v.as_object_mut())
            {
                if map.remove(server_name).is_some() {
                    changed = true;
                }
            }
        }
    }

    if changed {
        let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
        devzymer_core::config::write_atomic(target, format!("{text}\n").as_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

/// Path of the binary that would be removed by a full uninstall.
pub fn executable_path() -> Option<PathBuf> {
    devzymer_core::workspace::which("codegraph")
}
