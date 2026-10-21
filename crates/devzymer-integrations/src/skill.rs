//! A skill `devzymer`, com versão e propriedade dos arquivos.
//!
//! O conteúdo canônico vive neste crate (`assets/skill`) e é materializado nas
//! pastas de skills do projeto. A versão da skill é independente do esquema do
//! SQLite, da versão do parser, do resolvedor e do binário: elas se associam,
//! mas não se confundem.
//!
//! Cada instalação grava um manifesto com o hash de cada arquivo que o
//! devzymer escreveu. É o manifesto que separa "arquivo do devzymer, ainda
//! como foi instalado" (pode ser atualizado) de "arquivo que o usuário
//! alterou" (nunca é substituído).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::instructions::write_file_atomic;

pub const SKILL_NAME: &str = "devzymer";
/// A versão do conteúdo da skill. Sobe quando qualquer arquivo do modelo muda.
pub const SKILL_VERSION: &str = "1.0.0";
/// O protocolo MCP contra o qual a skill foi escrita.
pub const MCP_PROTOCOL: &str = "2025-06-18";
/// A menor versão do devzymer que expõe as ferramentas que a skill cita.
pub const MIN_CLI_VERSION: &str = "0.1.2";
pub const MANIFEST_FILE: &str = ".devzymer-manifest.json";

/// As ferramentas MCP que a skill cita.
pub const REQUIRED_TOOLS: &[&str] = &[
    "context_resolve",
    "context_read",
    "context_impact",
    "context_patch",
    "context_sync",
    "context_status",
];

/// Um arquivo do modelo canônico.
pub struct TemplateFile {
    pub path: &'static str,
    pub content: &'static str,
}

/// O modelo canônico, único no código-fonte.
pub const TEMPLATE: &[TemplateFile] = &[
    TemplateFile {
        path: "SKILL.md",
        content: include_str!("../assets/skill/SKILL.md"),
    },
    TemplateFile {
        path: "references/tool-selection.md",
        content: include_str!("../assets/skill/references/tool-selection.md"),
    },
    TemplateFile {
        path: "references/repository-discovery.md",
        content: include_str!("../assets/skill/references/repository-discovery.md"),
    },
    TemplateFile {
        path: "references/bug-investigation.md",
        content: include_str!("../assets/skill/references/bug-investigation.md"),
    },
    TemplateFile {
        path: "references/dependency-analysis.md",
        content: include_str!("../assets/skill/references/dependency-analysis.md"),
    },
    TemplateFile {
        path: "references/safe-editing.md",
        content: include_str!("../assets/skill/references/safe-editing.md"),
    },
    TemplateFile {
        path: "references/fallback-and-recovery.md",
        content: include_str!("../assets/skill/references/fallback-and-recovery.md"),
    },
    TemplateFile {
        path: "references/context-efficiency.md",
        content: include_str!("../assets/skill/references/context-efficiency.md"),
    },
];

/// O manifesto gravado ao lado da skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub manifest_schema: u32,
    pub name: String,
    pub skill_version: String,
    pub managed_by: String,
    pub min_cli_version: String,
    pub mcp_protocol: String,
    pub requires_tools: Vec<String>,
    /// Hash de cada arquivo, como o devzymer o escreveu.
    pub files: BTreeMap<String, String>,
}

/// As quebras de linha não participam do hash, para que `core.autocrlf` não
/// faça um arquivo intacto parecer alterado.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

pub fn hash_of(text: &str) -> String {
    let digest = blake3::hash(normalize(text).as_bytes());
    format!("blake3:{}", &digest.to_hex()[..32])
}

/// O estado de um arquivo da skill em relação ao modelo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Missing,
    /// Igual ao modelo atual.
    Current,
    /// Escrito pelo devzymer, intacto, de um modelo anterior: pode ser atualizado.
    Outdated,
    /// O devzymer o escreveu e o usuário o alterou depois.
    Modified,
    /// Existe e o devzymer não o escreveu.
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    Written,
    Updated,
    Unchanged,
    /// O arquivo é do usuário e foi mantido.
    Kept,
    /// Arquivo de um modelo anterior que deixou de existir.
    Removed,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOutcome {
    pub path: String,
    pub state: FileState,
    pub action: FileAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillAction {
    Created,
    Updated,
    Unchanged,
    /// Parte dos arquivos foi mantida porque o usuário os alterou.
    Partial,
    /// Nada foi feito: instalação mais nova ou manifesto inseguro.
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillOutcome {
    pub dir: PathBuf,
    pub action: SkillAction,
    pub version_before: Option<String>,
    pub version: String,
    pub files: Vec<FileOutcome>,
    pub conflicts: Vec<String>,
}

/// A path as shown to the user, with one separator on every platform.
fn shown(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(['-', '+']).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// True when `candidate` is a newer version than `reference`.
pub fn is_newer(candidate: &str, reference: &str) -> bool {
    match (parse_version(candidate), parse_version(reference)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The manifest the current template produces, for the given file hashes.
fn manifest_for(files: BTreeMap<String, String>) -> Manifest {
    Manifest {
        manifest_schema: 1,
        name: SKILL_NAME.to_string(),
        skill_version: SKILL_VERSION.to_string(),
        managed_by: "devzymer".to_string(),
        min_cli_version: MIN_CLI_VERSION.to_string(),
        mcp_protocol: MCP_PROTOCOL.to_string(),
        requires_tools: REQUIRED_TOOLS.iter().map(|t| t.to_string()).collect(),
        files,
    }
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    let path = dir.join(MANIFEST_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("manifesto ilegível ({error})")),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("manifesto inválido ({error})"))
}

fn read_normalized(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| normalize(&String::from_utf8_lossy(&bytes)))
}

fn state_of(actual: Option<&str>, canonical: &str, recorded: Option<&String>) -> FileState {
    let Some(actual) = actual else {
        return FileState::Missing;
    };
    if hash_of(actual) == hash_of(canonical) {
        return FileState::Current;
    }
    match recorded {
        Some(recorded) if *recorded == hash_of(actual) => FileState::Outdated,
        Some(_) => FileState::Modified,
        None => FileState::Foreign,
    }
}

/// Installs or updates the skill in `dir`.
///
/// With `apply` false nothing is written, and the outcome describes what
/// would happen.
pub fn install(dir: &Path, apply: bool) -> SkillOutcome {
    let mut outcome = SkillOutcome {
        dir: dir.to_path_buf(),
        action: SkillAction::Unchanged,
        version_before: None,
        version: SKILL_VERSION.to_string(),
        files: Vec::new(),
        conflicts: Vec::new(),
    };

    let manifest = match read_manifest(dir) {
        Ok(manifest) => manifest,
        Err(reason) => {
            outcome.conflicts.push(format!(
                "{}: {reason}; nenhum arquivo foi tratado como do devzymer",
                shown(&dir.join(MANIFEST_FILE))
            ));
            None
        }
    };
    outcome.version_before = manifest.as_ref().map(|m| m.skill_version.clone());

    if let Some(manifest) = &manifest {
        if is_newer(&manifest.skill_version, SKILL_VERSION) {
            outcome.action = SkillAction::Skipped;
            outcome.version = manifest.skill_version.clone();
            outcome.conflicts.push(format!(
                "{}: a skill instalada é a {}, mais nova que a {} deste devzymer; \
                 não será rebaixada. Atualize o devzymer",
                shown(dir),
                manifest.skill_version,
                SKILL_VERSION
            ));
            return outcome;
        }
    }

    let recorded: BTreeMap<String, String> = manifest
        .as_ref()
        .map(|m| m.files.clone())
        .unwrap_or_default();
    let existed = dir.join("SKILL.md").exists() || manifest.is_some();

    let mut new_files = recorded.clone();
    let mut wrote = false;
    let mut kept = false;

    for file in TEMPLATE {
        let target = dir.join(file.path);
        let actual = read_normalized(&target);
        let state = state_of(actual.as_deref(), file.content, recorded.get(file.path));
        let action = match state {
            FileState::Current => {
                new_files.insert(file.path.to_string(), hash_of(file.content));
                FileAction::Unchanged
            }
            FileState::Missing | FileState::Outdated => {
                if apply && write_file_atomic(&target, normalize(file.content).as_bytes()).is_err()
                {
                    outcome.conflicts.push(format!(
                        "{}: não foi possível gravar o arquivo",
                        shown(&target)
                    ));
                    kept = true;
                    FileAction::Kept
                } else {
                    new_files.insert(file.path.to_string(), hash_of(file.content));
                    wrote = true;
                    if state == FileState::Missing {
                        FileAction::Written
                    } else {
                        FileAction::Updated
                    }
                }
            }
            FileState::Modified | FileState::Foreign => {
                kept = true;
                let why = if state == FileState::Modified {
                    "foi alterado depois da instalação"
                } else {
                    "não foi escrito pelo devzymer"
                };
                outcome.conflicts.push(format!(
                    "{}: {why}; foi mantido. Para receber a versão {} do devzymer, \
                     mova ou remova o arquivo e rode `devzymer init --repair`",
                    shown(&target),
                    SKILL_VERSION
                ));
                FileAction::Kept
            }
        };
        outcome.files.push(FileOutcome {
            path: file.path.to_string(),
            state,
            action,
        });
    }

    // Files of an older template that no longer exist: removed only when the
    // user never touched them.
    for (path, hash) in &recorded {
        if TEMPLATE.iter().any(|f| f.path == path) {
            continue;
        }
        let target = dir.join(path);
        match read_normalized(&target) {
            Some(text) if hash_of(&text) == *hash => {
                if !apply || std::fs::remove_file(&target).is_ok() {
                    new_files.remove(path);
                    wrote = true;
                    outcome.files.push(FileOutcome {
                        path: path.clone(),
                        state: FileState::Outdated,
                        action: FileAction::Removed,
                    });
                }
            }
            Some(_) => {
                new_files.remove(path);
                outcome.conflicts.push(format!(
                    "{}: faz parte de uma versão anterior da skill e foi alterado; foi mantido",
                    shown(&target)
                ));
            }
            None => {
                new_files.remove(path);
            }
        }
    }

    let new_manifest = manifest_for(new_files);
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&new_manifest).unwrap_or_default()
    );
    let current = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap_or_default();
    let manifest_changes = normalize(&current) != normalize(&rendered);
    if manifest_changes && (wrote || manifest.is_none() || !kept) {
        if apply && write_file_atomic(&dir.join(MANIFEST_FILE), rendered.as_bytes()).is_err() {
            outcome.conflicts.push(format!(
                "{}: não foi possível gravar o manifesto",
                shown(&dir.join(MANIFEST_FILE))
            ));
        }
        wrote = true;
    }

    outcome.action = if kept && !wrote {
        SkillAction::Partial
    } else if !wrote {
        SkillAction::Unchanged
    } else if !existed {
        SkillAction::Created
    } else if kept {
        SkillAction::Partial
    } else {
        SkillAction::Updated
    };
    outcome
}

/// O resultado de remover a skill.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UninstallOutcome {
    pub removed: Vec<String>,
    /// Arquivos mantidos porque o usuário os alterou.
    pub kept: Vec<String>,
}

/// Removes the files the devzymer wrote and the user never touched.
///
/// Ownership comes from the manifest. A file that already equals the template
/// is removable even without a manifest, since deleting it loses nothing.
pub fn uninstall(dir: &Path) -> UninstallOutcome {
    let mut outcome = UninstallOutcome::default();
    if !dir.exists() {
        return outcome;
    }
    let recorded = read_manifest(dir)
        .ok()
        .flatten()
        .map(|m| m.files)
        .unwrap_or_default();
    let mut paths: Vec<String> = TEMPLATE.iter().map(|f| f.path.to_string()).collect();
    for path in recorded.keys() {
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }
    for path in paths {
        let target = dir.join(&path);
        let Some(actual) = read_normalized(&target) else {
            continue;
        };
        let canonical = TEMPLATE.iter().find(|f| f.path == path).map(|f| f.content);
        let owned = canonical.is_some_and(|c| hash_of(c) == hash_of(&actual))
            || recorded.get(&path).is_some_and(|h| *h == hash_of(&actual));
        if owned && std::fs::remove_file(&target).is_ok() {
            outcome.removed.push(path);
        } else {
            outcome.kept.push(path);
        }
    }
    if outcome.kept.is_empty() {
        let _ = std::fs::remove_file(dir.join(MANIFEST_FILE));
    }
    // Only directories that ended up empty go away.
    let _ = std::fs::remove_dir(dir.join("references"));
    let mut current = Some(dir);
    for _ in 0..3 {
        let Some(path) = current else { break };
        if std::fs::remove_dir(path).is_err() {
            break;
        }
        current = path.parent();
    }
    outcome
}

/// What is on disk, without changing anything.
#[derive(Debug, Clone, Serialize)]
pub struct Inspection {
    pub dir: PathBuf,
    pub installed: bool,
    pub version: Option<String>,
    pub files: Vec<(String, FileState)>,
    pub frontmatter_ok: bool,
    pub detail: String,
}

impl Inspection {
    /// True when every file matches the current template.
    pub fn is_current(&self) -> bool {
        self.installed
            && self.frontmatter_ok
            && self
                .files
                .iter()
                .all(|(_, state)| *state == FileState::Current)
    }
}

pub fn inspect(dir: &Path) -> Inspection {
    let manifest = read_manifest(dir).ok().flatten();
    let recorded = manifest
        .as_ref()
        .map(|m| m.files.clone())
        .unwrap_or_default();
    let files: Vec<(String, FileState)> = TEMPLATE
        .iter()
        .map(|file| {
            let actual = read_normalized(&dir.join(file.path));
            (
                file.path.to_string(),
                state_of(actual.as_deref(), file.content, recorded.get(file.path)),
            )
        })
        .collect();
    let skill_md = read_normalized(&dir.join("SKILL.md"));
    let (frontmatter_ok, detail) = match skill_md.as_deref() {
        None => (false, "SKILL.md ausente".to_string()),
        Some(text) => match validate_frontmatter(text, SKILL_NAME) {
            Ok(_) => (true, "frontmatter válido".to_string()),
            Err(reason) => (false, reason),
        },
    };
    Inspection {
        dir: dir.to_path_buf(),
        installed: skill_md.is_some(),
        version: manifest.map(|m| m.skill_version),
        files,
        frontmatter_ok,
        detail,
    }
}

/// The fields of a skill's frontmatter that hosts use to select it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: String,
    pub description: String,
}

/// Validates the YAML frontmatter of a `SKILL.md` against the Agent Skills
/// conventions: a lowercase hyphenated `name` equal to the directory, and a
/// non-empty `description` of at most 1024 characters.
pub fn validate_frontmatter(text: &str, directory: &str) -> Result<Frontmatter, String> {
    let text = normalize(text);
    let body = text
        .strip_prefix("---\n")
        .ok_or("o SKILL.md precisa começar com `---`")?;
    let end = body
        .find("\n---\n")
        .or_else(|| body.strip_suffix("\n---").map(|rest| rest.len()))
        .ok_or("o frontmatter não termina com `---`")?;
    let yaml = &body[..end];

    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in yaml.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            return Err("o frontmatter usa tabulação; o YAML exige espaços".to_string());
        }
        if line.starts_with(' ') {
            let key = current
                .clone()
                .ok_or("linha indentada sem chave no frontmatter")?;
            let entry = fields.entry(key).or_default();
            if !entry.is_empty() {
                entry.push(' ');
            }
            entry.push_str(line.trim());
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("linha inválida no frontmatter: `{line}`"))?;
        let key = key.trim().to_string();
        let value = value.trim();
        let value = if matches!(value, ">" | ">-" | "|" | "|-" | ">+" | "|+") {
            ""
        } else {
            value
        };
        fields.insert(key.clone(), value.trim_matches('"').to_string());
        current = Some(key);
    }

    let name = fields.get("name").cloned().unwrap_or_default();
    let description = fields.get("description").cloned().unwrap_or_default();
    let valid_name = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid_name {
        return Err(format!(
            "`name` inválido ({name:?}): use minúsculas, números e hífens, até 64 caracteres"
        ));
    }
    if name != directory {
        return Err(format!(
            "`name` ({name}) precisa ser igual ao nome da pasta ({directory})"
        ));
    }
    if description.is_empty() {
        return Err("`description` está vazia".to_string());
    }
    if description.chars().count() > 1024 {
        return Err("`description` passa de 1024 caracteres".to_string());
    }
    Ok(Frontmatter { name, description })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-skill-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn template(path: &str) -> &'static str {
        TEMPLATE.iter().find(|f| f.path == path).unwrap().content
    }

    #[test]
    fn the_template_has_a_valid_frontmatter() {
        let front = validate_frontmatter(template("SKILL.md"), SKILL_NAME).unwrap();
        assert_eq!(front.name, "devzymer");
        assert!(front.description.contains("devzymer MCP"));
        assert!(front.description.chars().count() <= 1024);
        assert!(front.description.contains("Not for running tests"));
    }

    #[test]
    fn a_broken_frontmatter_is_reported() {
        for (text, needle) in [
            ("sem frontmatter", "começar"),
            ("---\nname: Devzymer\ndescription: x\n---\n", "inválido"),
            ("---\nname: outro\ndescription: x\n---\n", "pasta"),
            ("---\nname: devzymer\n---\n", "vazia"),
            ("---\nname: devzymer\ndescription: x\n", "termina"),
            ("---\nname: devzymer\n\tdescription: x\n---\n", "tabulação"),
        ] {
            let error = validate_frontmatter(text, "devzymer").unwrap_err();
            assert!(error.contains(needle), "{text:?} -> {error}");
        }
    }

    #[test]
    fn the_main_skill_stays_short_and_indexes_every_reference() {
        let main = template("SKILL.md");
        assert!(main.lines().count() <= 130, "SKILL.md deve ser enxuto");
        for file in TEMPLATE.iter().filter(|f| f.path != "SKILL.md") {
            assert!(
                main.contains(file.path),
                "{} sem índice no SKILL.md",
                file.path
            );
            for other in TEMPLATE.iter().filter(|o| o.path != "SKILL.md") {
                let name = other.path.trim_start_matches("references/");
                assert!(
                    other.path == file.path || !file.content.contains(name),
                    "{} aponta para {}: sem cadeias entre referências",
                    file.path,
                    other.path
                );
            }
        }
    }

    #[test]
    fn the_template_cites_only_known_tools() {
        let main = template("SKILL.md");
        for tool in REQUIRED_TOOLS {
            assert!(main.contains(tool), "{tool}");
        }
        assert!(main.contains(&format!("devzymer-skill-version: {SKILL_VERSION}")));
    }

    #[test]
    fn a_fresh_install_writes_everything_and_a_second_run_changes_nothing() {
        let dir = scratch("fresh").join("devzymer");
        let first = install(&dir, true);
        assert_eq!(first.action, SkillAction::Created);
        assert!(dir.join("SKILL.md").exists());
        assert!(dir.join("references/safe-editing.md").exists());
        assert!(dir.join(MANIFEST_FILE).exists());
        let before = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();

        let second = install(&dir, true);
        assert_eq!(second.action, SkillAction::Unchanged);
        assert!(second.conflicts.is_empty());
        assert_eq!(
            before,
            std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap()
        );
        assert!(inspect(&dir).is_current());
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let dir = scratch("dry").join("devzymer");
        let outcome = install(&dir, false);
        assert_eq!(outcome.action, SkillAction::Created);
        assert!(!dir.exists());
    }

    #[test]
    fn a_user_edit_is_kept_and_reported() {
        let dir = scratch("edited").join("devzymer");
        install(&dir, true);
        let skill = dir.join("SKILL.md");
        let edited = format!(
            "{}\n## Meu fluxo\nusar sempre o módulo X\n",
            template("SKILL.md")
        );
        std::fs::write(&skill, &edited).unwrap();

        let outcome = install(&dir, true);
        assert_eq!(outcome.action, SkillAction::Partial);
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
        assert!(outcome.conflicts.iter().any(|c| c.contains("alterado")));
        let again = install(&dir, true);
        assert_eq!(std::fs::read_to_string(&skill).unwrap(), edited);
        assert_eq!(again.action, SkillAction::Partial);
    }

    #[test]
    fn a_file_the_devzymer_did_not_write_is_never_replaced() {
        let dir = scratch("foreign").join("devzymer");
        std::fs::create_dir_all(&dir).unwrap();
        let mine = "---\nname: devzymer\ndescription: minha própria skill\n---\ncorpo\n";
        std::fs::write(dir.join("SKILL.md"), mine).unwrap();

        let outcome = install(&dir, true);
        assert_eq!(std::fs::read_to_string(dir.join("SKILL.md")).unwrap(), mine);
        assert!(outcome
            .conflicts
            .iter()
            .any(|c| c.contains("não foi escrito")));
        assert!(dir.join("references/tool-selection.md").exists());
    }

    #[test]
    fn an_outdated_untouched_template_is_upgraded_and_custom_files_survive() {
        let dir = scratch("upgrade").join("devzymer");
        install(&dir, true);
        let old = "---\nname: devzymer\ndescription: versão antiga\n---\ncorpo antigo\n";
        std::fs::write(dir.join("SKILL.md"), old).unwrap();
        let mut manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap())
                .unwrap();
        manifest.skill_version = "0.9.0".to_string();
        manifest.files.insert("SKILL.md".to_string(), hash_of(old));
        manifest
            .files
            .insert("references/legado.md".to_string(), hash_of("legado"));
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("references/legado.md"), "legado").unwrap();
        std::fs::write(dir.join("references/meu-fluxo.md"), "meu fluxo").unwrap();

        let outcome = install(&dir, true);
        assert_eq!(
            outcome.action,
            SkillAction::Updated,
            "{:?}",
            outcome.conflicts
        );
        assert_eq!(outcome.version_before.as_deref(), Some("0.9.0"));
        assert_eq!(
            normalize(&std::fs::read_to_string(dir.join("SKILL.md")).unwrap()),
            normalize(template("SKILL.md"))
        );
        assert!(!dir.join("references/legado.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("references/meu-fluxo.md")).unwrap(),
            "meu fluxo"
        );
        assert_eq!(install(&dir, true).action, SkillAction::Unchanged);
    }

    #[test]
    fn a_newer_installed_skill_is_not_downgraded() {
        let dir = scratch("newer").join("devzymer");
        install(&dir, true);
        let mut manifest: Manifest =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap())
                .unwrap();
        manifest.skill_version = "9.0.0".to_string();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("SKILL.md"), "conteúdo futuro").unwrap();

        let outcome = install(&dir, true);
        assert_eq!(outcome.action, SkillAction::Skipped);
        assert_eq!(
            std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            "conteúdo futuro"
        );
    }

    #[test]
    fn line_endings_do_not_make_an_intact_file_look_edited() {
        let dir = scratch("crlf").join("devzymer");
        install(&dir, true);
        let skill = dir.join("SKILL.md");
        let crlf = std::fs::read_to_string(&skill)
            .unwrap()
            .replace('\n', "\r\n");
        std::fs::write(&skill, crlf).unwrap();
        let outcome = install(&dir, true);
        assert_eq!(outcome.action, SkillAction::Unchanged);
        assert!(outcome.conflicts.is_empty());
    }

    #[test]
    fn a_missing_file_is_restored() {
        let dir = scratch("restore").join("devzymer");
        install(&dir, true);
        std::fs::remove_file(dir.join("references/context-efficiency.md")).unwrap();
        let outcome = install(&dir, true);
        assert_eq!(outcome.action, SkillAction::Updated);
        assert!(dir.join("references/context-efficiency.md").exists());
    }

    #[test]
    fn a_corrupt_manifest_treats_nothing_as_owned() {
        let dir = scratch("badmanifest").join("devzymer");
        install(&dir, true);
        std::fs::write(dir.join(MANIFEST_FILE), "{ não é json").unwrap();
        std::fs::write(dir.join("SKILL.md"), "editado").unwrap();
        let outcome = install(&dir, true);
        assert_eq!(
            std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            "editado"
        );
        assert!(outcome.conflicts.iter().any(|c| c.contains("manifesto")));
    }

    #[test]
    fn uninstall_removes_only_what_the_user_left_alone() {
        let dir = scratch("uninstall").join(".agents/skills/devzymer");
        install(&dir, true);
        std::fs::write(dir.join("references/meu-fluxo.md"), "meu").unwrap();
        let edited = format!("{}\nextra\n", template("references/safe-editing.md"));
        std::fs::write(dir.join("references/safe-editing.md"), &edited).unwrap();

        let outcome = uninstall(&dir);
        assert!(outcome
            .kept
            .contains(&"references/safe-editing.md".to_string()));
        assert!(!dir.join("SKILL.md").exists());
        assert!(
            dir.join(MANIFEST_FILE).exists(),
            "o manifesto fica enquanto sobrar arquivo seu"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("references/safe-editing.md")).unwrap(),
            edited
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("references/meu-fluxo.md")).unwrap(),
            "meu"
        );
    }

    #[test]
    fn uninstall_leaves_no_directory_behind_when_nothing_is_kept() {
        let base = scratch("uninstall-clean");
        let dir = base.join(".claude/skills/devzymer");
        install(&dir, true);
        let outcome = uninstall(&dir);
        assert!(outcome.kept.is_empty());
        assert!(!dir.exists());
        assert!(!base.join(".claude").exists());
    }

    #[test]
    fn versions_are_compared_numerically() {
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("lixo", "1.0.0"));
    }
}
