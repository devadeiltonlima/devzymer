//! Blocos gerenciados nos arquivos de instruções do projeto.
//!
//! `CLAUDE.md` e `AGENTS.md` pertencem ao usuário. O devzymer só escreve entre
//! os seus marcadores, e o que está fora deles nunca é lido como parte do
//! bloco nem reescrito.
//!
//! O planejamento é puro: recebe os bytes atuais e devolve os bytes novos, sem
//! tocar o disco. Marcadores malformados deixam o arquivo como está, porque
//! adivinhar onde um bloco termina poderia apagar texto do usuário.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::claude::{ROUTING_POLICY, RULES_END, RULES_START};

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// O que o planejamento decidiu fazer com o arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockAction {
    /// O arquivo não existia e será criado com o bloco.
    Created,
    /// O arquivo existia sem bloco e o bloco foi acrescentado ao fim.
    Appended,
    /// O bloco existia e foi atualizado no mesmo lugar.
    Updated,
    /// O arquivo já está como deveria.
    Unchanged,
    /// O arquivo não foi alterado porque seria inseguro.
    Skipped,
}

/// O resultado do planejamento de um arquivo de instruções.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub action: BlockAction,
    /// Os bytes que o arquivo deve ter. Igual aos atuais quando nada muda.
    pub content: Vec<u8>,
    /// Avisos para o usuário: conflitos, duplicatas removidas, recusas.
    pub warnings: Vec<String>,
}

impl Plan {
    /// True when the plan changes the file.
    pub fn writes(&self) -> bool {
        matches!(
            self.action,
            BlockAction::Created | BlockAction::Appended | BlockAction::Updated
        )
    }
}

struct Marker {
    start: bool,
    /// Início da linha do marcador.
    begin: usize,
    /// Fim da linha sem o terminador.
    end: usize,
    /// Fim da linha com o terminador.
    end_with_break: usize,
}

/// Encontra os marcadores que ocupam uma linha inteira.
///
/// Um marcador citado no meio de uma frase ou de um trecho de código não
/// conta: só o marcador sozinho na linha delimita o bloco.
fn markers_of(text: &str) -> Vec<Marker> {
    let mut found = Vec::new();
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let kind = if trimmed == RULES_START {
            Some(true)
        } else if trimmed == RULES_END {
            Some(false)
        } else {
            None
        };
        if let Some(start) = kind {
            let without_break = line.trim_end_matches(['\n', '\r']);
            found.push(Marker {
                start,
                begin: offset,
                end: offset + without_break.len(),
                end_with_break: offset + line.len(),
            });
        }
        offset += line.len();
    }
    found
}

/// Pairs the markers, or describes why they cannot be paired.
fn pair_markers(markers: &[Marker]) -> Result<Vec<(&Marker, &Marker)>, String> {
    let mut pairs = Vec::new();
    let mut open: Option<&Marker> = None;
    for marker in markers {
        match (open, marker.start) {
            (None, true) => open = Some(marker),
            (Some(start), false) => {
                pairs.push((start, marker));
                open = None;
            }
            (None, false) => return Err("há um marcador de fim sem marcador de início".to_string()),
            (Some(_), true) => {
                return Err("há um marcador de início sem marcador de fim".to_string())
            }
        }
    }
    if open.is_some() {
        return Err("há um marcador de início sem marcador de fim".to_string());
    }
    Ok(pairs)
}

fn newline_of(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    if crlf > 0 && crlf * 2 >= lf {
        "\r\n"
    } else {
        "\n"
    }
}

fn render(block: &str, newline: &str) -> String {
    let block = block.trim_end_matches(['\n', '\r']);
    if newline == "\n" {
        block.to_string()
    } else {
        block.replace('\n', newline)
    }
}

/// Plans the content of an instructions file that must carry `block`.
pub fn plan_block(existing: Option<&[u8]>, block: &str) -> Plan {
    let Some(bytes) = existing else {
        let mut content = render(block, "\n").into_bytes();
        content.push(b'\n');
        return Plan {
            action: BlockAction::Created,
            content,
            warnings: Vec::new(),
        };
    };

    let (bom, body) = match bytes.strip_prefix(BOM) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    let Ok(text) = std::str::from_utf8(body) else {
        return skipped(
            bytes,
            "o arquivo não está em UTF-8; nada foi alterado para não corromper o texto",
        );
    };

    let newline = newline_of(text);
    let rendered = render(block, newline);
    let markers = markers_of(text);
    let pairs = match pair_markers(&markers) {
        Ok(pairs) => pairs,
        Err(reason) => {
            return skipped(
                bytes,
                &format!(
                    "marcadores do devzymer malformados ({reason}); o arquivo não foi alterado. \
                     Corrija ou remova o marcador e rode `devzymer init` de novo"
                ),
            )
        }
    };

    let mut warnings = Vec::new();
    let mut out = String::with_capacity(text.len() + rendered.len() + 8);
    let action;
    if pairs.is_empty() {
        out.push_str(text);
        if !out.is_empty() {
            if !out.ends_with('\n') {
                out.push_str(newline);
            }
            if !(out.ends_with("\n\n") || out.ends_with("\r\n\r\n")) {
                out.push_str(newline);
            }
        }
        out.push_str(&rendered);
        out.push_str(newline);
        action = BlockAction::Appended;
        warnings.extend(conflicts_outside(text, &[]));
    } else {
        let mut cursor = 0usize;
        for (index, (start, end)) in pairs.iter().enumerate() {
            out.push_str(&text[cursor..start.begin]);
            if index == 0 {
                out.push_str(&rendered);
                cursor = end.end;
            } else {
                cursor = end.end_with_break;
            }
        }
        out.push_str(&text[cursor..]);
        if pairs.len() > 1 {
            warnings.push(format!(
                "havia {} blocos do devzymer; só um foi mantido",
                pairs.len()
            ));
        }
        let regions: Vec<(usize, usize)> = pairs
            .iter()
            .map(|(start, end)| (start.begin, end.end_with_break))
            .collect();
        warnings.extend(conflicts_outside(text, &regions));
        action = BlockAction::Updated;
    }

    let mut content = Vec::with_capacity(out.len() + 3);
    if bom {
        content.extend_from_slice(BOM);
    }
    content.extend_from_slice(out.as_bytes());
    if content == bytes {
        return Plan {
            action: BlockAction::Unchanged,
            content: bytes.to_vec(),
            warnings,
        };
    }
    Plan {
        action,
        content,
        warnings,
    }
}

fn skipped(bytes: &[u8], reason: &str) -> Plan {
    Plan {
        action: BlockAction::Skipped,
        content: bytes.to_vec(),
        warnings: vec![reason.to_string()],
    }
}

/// Removes the managed block, keeping everything else. Returns `None` when
/// there is no block or the markers cannot be paired.
pub fn remove_block(text: &str) -> Option<String> {
    let markers = markers_of(text);
    let pairs = pair_markers(&markers).ok()?;
    if pairs.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (start, end) in &pairs {
        out.push_str(&text[cursor..start.begin]);
        cursor = end.end_with_break;
    }
    out.push_str(&text[cursor..]);
    Some(out)
}

const NEGATIONS: &[&str] = &[
    "do not use",
    "don't use",
    "never use",
    "avoid",
    "ignore",
    "disable",
    "stop using",
    "não use",
    "nao use",
    "não utilize",
    "nao utilize",
    "nunca use",
    "evite",
    "desative",
];

const SEARCH_PREFERENCES: &[&str] = &[
    "always use grep",
    "always use rg",
    "prefer grep",
    "use grep first",
    "sempre use grep",
    "sempre use o grep",
    "prefira o grep",
    "prefira grep",
];

/// Lines written by the user outside the managed block that pull the other way.
///
/// They are reported and never changed: the user wrote them on purpose.
fn conflicts_outside(text: &str, regions: &[(usize, usize)]) -> Vec<String> {
    let mut found = Vec::new();
    let mut offset = 0usize;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += line.len();
        if regions
            .iter()
            .any(|(begin, end)| start >= *begin && start < *end)
        {
            continue;
        }
        let lower = line.to_lowercase();
        let mentions = lower.contains("devzymer")
            || lower.contains("context_resolve")
            || lower.contains("context_read")
            || lower.contains("context_patch");
        let negated = NEGATIONS.iter().any(|word| lower.contains(word));
        let prefers_grep = SEARCH_PREFERENCES.iter().any(|word| lower.contains(word));
        if (mentions && negated) || prefers_grep {
            let snippet: String = line.trim().chars().take(110).collect();
            found.push(format!(
                "linha {}: instrução própria que pode conflitar com o devzymer: «{snippet}». \
                 Foi mantida; o devzymer não a sobrescreve",
                index + 1
            ));
        }
    }
    found
}

/// The block written into `CLAUDE.md`.
pub fn claude_block() -> String {
    block(
        " (`mcp__devzymer__context_resolve`)",
        "- Bug, análise de impacto ou repositório desconhecido: invoque a skill `devzymer` \
         (ferramenta `Skill`) antes de explorar; ela traz o protocolo de investigação.",
    )
}

/// The block written into `AGENTS.md`, read by Codex and OpenCode.
///
/// It avoids tool names that exist in one host only.
pub fn agents_block() -> String {
    block(
        " (no OpenCode, `devzymer_context_resolve`)",
        "- Bug, análise de impacto ou repositório desconhecido: invoque a skill `devzymer` \
         (`.agents/skills/devzymer/`; no Codex, `$devzymer`) antes de explorar; ela traz o \
         protocolo de investigação.",
    )
}

fn block(tool_note: &str, skill_line: &str) -> String {
    format!(
        "{RULES_START}
## devzymer

Este repositório está indexado localmente pelo devzymer e exposto pelo servidor MCP `devzymer`.

{ROUTING_POLICY}

- Localizar código: comece por `context_resolve`{tool_note} com o pedido do usuário. Leia só a \
faixa devolvida com `context_read`, símbolo antes de arquivo inteiro.
- Dependências e impacto: `context_impact`. Aresta heurística não é dependência confirmada, e \
aresta ausente não prova que não há chamador.
- Editar: confira o trecho lido antes. `context_patch` exige o `expected_sha` da leitura; as \
ferramentas nativas de edição continuam válidas.
- Depois de criar, renomear ou apagar arquivos: `context_sync`.
- `nao_encontrado:` não prova ausência: confirme com busca nativa. `ambiguidade:` pede o nome \
qualificado (`Classe.metodo`).
{skill_line}
- Índice ausente, desatualizado ou insuficiente, logs e saída de comandos: use as ferramentas \
nativas e diga que o fez. Não rode `devzymer init` na rotina: só se o projeto não estiver \
inicializado e o usuário autorizar, ou para um reparo autorizado.
- Pedidos explícitos do usuário e as permissões do host valem mais do que estas preferências.
{RULES_END}"
    )
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes a file through a temporary sibling and a rename.
///
/// The temporary name is unique per call, so two processes never share one,
/// and the rename is retried because OneDrive and antivirus scanners briefly
/// hold files on Windows. The permissions of an existing file are kept.
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "devzymer".to_string());
    let tmp = parent.join(format!(
        ".{name}.{}.{}.devzymer-tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        let mut attempt = 0;
        loop {
            match std::fs::rename(&tmp, path) {
                Ok(()) => return Ok(()),
                Err(error) if attempt < 5 => {
                    attempt += 1;
                    let _ = error;
                    std::thread::sleep(std::time::Duration::from_millis(40 * attempt));
                }
                Err(error) => return Err(error),
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_text() -> String {
        claude_block()
    }

    fn apply(existing: Option<&str>) -> Plan {
        plan_block(existing.map(str::as_bytes), &block_text())
    }

    fn text(plan: &Plan) -> String {
        String::from_utf8(plan.content.clone()).unwrap()
    }

    #[test]
    fn a_missing_file_is_created_with_the_block() {
        let plan = apply(None);
        assert_eq!(plan.action, BlockAction::Created);
        let content = text(&plan);
        assert!(content.starts_with(RULES_START));
        assert!(content.ends_with(&format!("{RULES_END}\n")));
    }

    #[test]
    fn user_content_is_preserved_and_the_block_is_appended() {
        let user = "# Meu projeto\n\nUse tabs, nunca espaços.\n";
        let plan = apply(Some(user));
        assert_eq!(plan.action, BlockAction::Appended);
        let content = text(&plan);
        assert!(content.starts_with(user));
        assert_eq!(content.matches(RULES_START).count(), 1);
        assert!(content.contains(&format!("{user}\n{RULES_START}")));
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let first = text(&apply(Some("# Projeto\n\nregra\n")));
        let second = apply(Some(&first));
        assert_eq!(second.action, BlockAction::Unchanged);
        assert_eq!(second.content, first.as_bytes());
    }

    #[test]
    fn an_outdated_block_is_replaced_in_place() {
        let old = format!("antes\n\n{RULES_START}\nregra velha\n{RULES_END}\n\ndepois\n");
        let plan = apply(Some(&old));
        assert_eq!(plan.action, BlockAction::Updated);
        let content = text(&plan);
        assert!(content.starts_with("antes\n\n"));
        assert!(content.ends_with("\n\ndepois\n"));
        assert!(!content.contains("regra velha"));
        assert_eq!(content.matches(RULES_START).count(), 1);
    }

    #[test]
    fn duplicated_blocks_collapse_into_one() {
        let doubled =
            format!("a\n{RULES_START}\nx\n{RULES_END}\nb\n{RULES_START}\ny\n{RULES_END}\nc\n");
        let plan = apply(Some(&doubled));
        let content = text(&plan);
        assert_eq!(content.matches(RULES_START).count(), 1);
        assert_eq!(content.matches(RULES_END).count(), 1);
        for kept in ["a\n", "b\n", "c\n"] {
            assert!(content.contains(kept), "{kept}");
        }
        assert!(plan.warnings.iter().any(|w| w.contains("2 blocos")));
    }

    #[test]
    fn a_start_without_an_end_leaves_the_file_alone() {
        let broken = format!("# Projeto\n{RULES_START}\ntexto do usuário depois\n");
        let plan = apply(Some(&broken));
        assert_eq!(plan.action, BlockAction::Skipped);
        assert_eq!(plan.content, broken.as_bytes());
        assert!(plan.warnings[0].contains("malformados"));
    }

    #[test]
    fn an_end_before_a_start_leaves_the_file_alone() {
        let broken = format!("{RULES_END}\nmeio\n{RULES_START}\n");
        let plan = apply(Some(&broken));
        assert_eq!(plan.action, BlockAction::Skipped);
        assert_eq!(plan.content, broken.as_bytes());
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let user = "# Projeto\r\n\r\nregra\r\n";
        let first = apply(Some(user));
        let content = text(&first);
        assert!(content.starts_with(user));
        assert_eq!(
            content.matches('\n').count(),
            content.matches("\r\n").count()
        );
        assert_eq!(apply(Some(&content)).action, BlockAction::Unchanged);
    }

    #[test]
    fn a_byte_order_mark_is_kept() {
        let mut bytes = BOM.to_vec();
        bytes.extend_from_slice(b"# Projeto\n");
        let plan = plan_block(Some(&bytes), &block_text());
        assert!(plan.content.starts_with(BOM));
        assert_eq!(plan.content[3..6], *b"# P");
        assert_eq!(
            plan_block(Some(&plan.content), &block_text()).action,
            BlockAction::Unchanged
        );
    }

    #[test]
    fn a_file_that_is_not_utf8_is_not_touched() {
        let bytes = [0x23, 0x20, 0xFF, 0xFE, 0x0A];
        let plan = plan_block(Some(&bytes), &block_text());
        assert_eq!(plan.action, BlockAction::Skipped);
        assert_eq!(plan.content, bytes);
    }

    #[test]
    fn a_marker_quoted_in_prose_is_not_a_marker() {
        let doc = format!("Os marcadores são `{RULES_START}` e `{RULES_END}`.\n");
        let plan = apply(Some(&doc));
        assert_eq!(plan.action, BlockAction::Appended);
        assert!(text(&plan).starts_with(&doc));
    }

    #[test]
    fn a_conflicting_user_rule_is_reported_and_kept() {
        let user = "# Regras\n\nNunca use o devzymer neste repositório.\nSempre use grep.\n";
        let plan = apply(Some(user));
        assert!(text(&plan).starts_with(user));
        assert_eq!(plan.warnings.len(), 2, "{:?}", plan.warnings);
        assert!(plan.warnings[0].contains("linha 3"));
    }

    #[test]
    fn the_text_of_the_block_itself_is_not_a_conflict() {
        let first = text(&apply(Some("# x\n")));
        assert!(apply(Some(&first)).warnings.is_empty());
    }

    #[test]
    fn removing_the_block_keeps_the_rest() {
        let with = text(&apply(Some("# Projeto\n\nregra\n")));
        let without = remove_block(&with).unwrap();
        assert!(without.contains("# Projeto"));
        assert!(without.contains("regra"));
        assert!(!without.contains(RULES_START));
        assert!(remove_block("sem bloco\n").is_none());
        assert!(remove_block(&format!("{RULES_START}\nsó início\n")).is_none());
    }

    #[test]
    fn both_blocks_state_the_policy_and_name_their_skill() {
        for (block, skill) in [
            (claude_block(), "ferramenta `Skill`"),
            (agents_block(), ".agents/skills/devzymer"),
        ] {
            assert!(block.contains(ROUTING_POLICY));
            assert!(block.contains(skill));
            assert!(block.contains("nao_encontrado:"));
            assert!(!block.contains("prioridade"));
            assert!(block.lines().count() <= 20, "o bloco deve ser curto");
        }
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join("devzymer-instructions-atomic");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("CLAUDE.md");
        std::fs::write(&target, "velho").unwrap();
        write_file_atomic(&target, b"novo").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "novo");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".devzymer-tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
