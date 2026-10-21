//! O estágio da inicialização, gravado em `.devzymer/init-state.json`.
//!
//! O `init` percorre etapas: indexa, verifica, grava os arquivos do agente.
//! Se ele for interrompido no meio, o arquivo diz até onde foi, e o `doctor`
//! e o `status` explicam o que falta em vez de deixar um projeto que parece
//! pronto sem estar. Os arquivos do agente só são gravados depois que o índice
//! foi verificado, então a presença deles nunca implica um índice saudável.

use std::path::Path;

use devzymer_core::paths;
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "init-state.json";

/// As etapas, na ordem em que acontecem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Indexação iniciada e ainda não concluída.
    Indexing,
    /// Índice e grafo gravados, ainda sem verificação.
    Indexed,
    /// Índice verificado (banco, FTS5, busca e MCP).
    Verified,
    /// Registro dos agentes e arquivos do projeto gravados, com pendências.
    AgentFiles,
    /// Tudo concluído.
    Complete,
}

impl Stage {
    pub fn describe(self) -> &'static str {
        match self {
            Stage::Indexing => "a indexação não terminou",
            Stage::Indexed => "o índice foi gravado, mas não foi verificado",
            Stage::Verified => {
                "o índice foi verificado, mas os arquivos do agente não foram gravados"
            }
            Stage::AgentFiles => "os arquivos do agente foram gravados com pendências",
            Stage::Complete => "inicialização completa",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitState {
    pub schema: u32,
    pub stage: Stage,
    pub devzymer_version: String,
    pub parser_version: String,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub note: Option<String>,
}

pub fn path(root: &Path) -> std::path::PathBuf {
    paths::project_dir(root).join(STATE_FILE)
}

/// Records the stage. The state is advisory, so a failure to write it never
/// stops the initialization.
pub fn write(root: &Path, stage: Stage, note: Option<&str>) {
    let state = InitState {
        schema: 1,
        stage,
        devzymer_version: env!("CARGO_PKG_VERSION").to_string(),
        parser_version: devzymer_core::store::PARSER_VERSION.to_string(),
        updated_at_ms: devzymer_core::util::now_millis() as u64,
        note: note.map(str::to_string),
    };
    if let Ok(text) = serde_json::to_string_pretty(&state) {
        let _ = devzymer_integrations::instructions::write_file_atomic(
            &path(root),
            format!("{text}\n").as_bytes(),
        );
    }
}

pub fn read(root: &Path) -> Option<InitState> {
    let text = std::fs::read_to_string(path(root)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_round_trips_and_a_missing_file_reads_as_none() {
        let root = std::env::temp_dir().join("devzymer-init-state");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(read(&root).is_none());
        write(&root, Stage::Verified, Some("nota"));
        let state = read(&root).unwrap();
        assert_eq!(state.stage, Stage::Verified);
        assert_eq!(state.note.as_deref(), Some("nota"));
        write(&root, Stage::Complete, None);
        assert_eq!(read(&root).unwrap().stage, Stage::Complete);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreadable_state_is_ignored() {
        let root = std::env::temp_dir().join("devzymer-init-state-bad");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(paths::project_dir(&root)).unwrap();
        std::fs::write(path(&root), "{ lixo").unwrap();
        assert!(read(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
