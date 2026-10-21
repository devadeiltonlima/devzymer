use std::path::Path;

use devzymer_core::config::write_atomic;
use devzymer_core::{Error, Result};

#[derive(Debug, Clone)]
pub struct PatchRequest {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub replacement: String,
    /// The revision the caller saw. It is mandatory.
    pub expected_sha: String,
    pub dry_run: bool,
    /// Authorizes the replacement of the whole file.
    pub allow_whole_file: bool,
}

/// The excerpt the caller read before patching.
///
/// A patch may only address a block inside the excerpt that was read, so a
/// range that came from a wider resolution cannot silently replace a block
/// the caller never saw.
#[derive(Debug, Clone)]
pub struct ReadBound {
    pub start_line: u32,
    pub end_line: u32,
    pub sha: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PatchOutcome {
    pub path: String,
    pub previous_sha: String,
    pub new_sha: String,
    pub lines_before: u32,
    pub lines_after: u32,
    pub replaced_lines: u32,
    pub dry_run: bool,
    pub diff_preview: String,
}

/// Applies a line range replacement.
///
/// The edit is refused unless the file still holds the revision the caller
/// read, the range lies inside the excerpt that was read, and the range
/// exists in the file. The bytes outside the range are preserved exactly,
/// including the line endings.
pub fn apply_patch(
    root: &Path,
    request: &PatchRequest,
    bound: Option<&ReadBound>,
) -> Result<PatchOutcome> {
    let abs = devzymer_core::util::resolve_inside(root, Path::new(&request.path))
        .map_err(|error| Error::PatchOutsideWorkspace(error.to_string()))?;
    let bytes = std::fs::read(&abs).map_err(|e| devzymer_core::Error::io(&abs, e))?;
    let previous_sha = short_hash(&bytes);

    let expected = request.expected_sha.trim();
    if expected.is_empty() {
        return Err(Error::PatchContentMismatch(format!(
            "{} exige expected_sha, o hash devolvido pela ultima leitura",
            request.path
        )));
    }
    if expected != previous_sha {
        return Err(Error::PatchStaleRevision(format!(
            "{} mudou depois da leitura: esperado {expected}, encontrado {previous_sha}",
            request.path
        )));
    }

    if let Some(bound) = bound {
        if bound.sha != previous_sha {
            return Err(Error::PatchStaleRevision(format!(
                "a leitura de {} e anterior a versao atual do arquivo",
                request.path
            )));
        }
        if request.start_line < bound.start_line || request.end_line > bound.end_line {
            return Err(Error::PatchAmbiguousTarget(format!(
                "o intervalo {}:{} esta fora do trecho lido ({}:{}); releia o bloco antes de alterar",
                request.start_line, request.end_line, bound.start_line, bound.end_line
            )));
        }
    }

    let text = String::from_utf8_lossy(&bytes).into_owned();
    let crlf = text.contains("\r\n");
    // The lines keep their original terminators, so the bytes outside the
    // range are preserved exactly.
    let parts: Vec<&str> = text.split_inclusive('\n').collect();
    let total = parts.len() as u32;
    if total == 0 {
        return Err(Error::PatchAmbiguousTarget(format!(
            "{} esta vazio e nao tem bloco para substituir",
            request.path
        )));
    }
    if request.start_line == 0 || request.start_line > total {
        return Err(Error::PatchAmbiguousTarget(format!(
            "a linha inicial {} esta fora do arquivo (1..{total})",
            request.start_line
        )));
    }
    if request.end_line < request.start_line {
        return Err(Error::PatchAmbiguousTarget(format!(
            "a linha final {} e anterior a inicial {}",
            request.end_line, request.start_line
        )));
    }
    if request.end_line > total {
        return Err(Error::PatchAmbiguousTarget(format!(
            "a linha final {} esta fora do arquivo (1..{total}); o arquivo mudou",
            request.end_line
        )));
    }
    if request.start_line == 1 && request.end_line == total && !request.allow_whole_file {
        return Err(Error::PatchAmbiguousTarget(
            "a substituicao do arquivo inteiro exige allow_whole_file; o padrao e uma \
             edicao minima"
                .to_string(),
        ));
    }

    let replacement_lines: Vec<&str> = request
        .replacement
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let replacement_lines: Vec<&str> =
        if replacement_lines.len() == 1 && replacement_lines[0].is_empty() {
            Vec::new()
        } else {
            replacement_lines
        };

    let ending = if crlf { "\r\n" } else { "\n" };
    let mut updated = String::with_capacity(bytes.len() + request.replacement.len());
    for part in &parts[..(request.start_line as usize - 1)] {
        updated.push_str(part);
    }
    for (index, line) in replacement_lines.iter().enumerate() {
        if index > 0 {
            updated.push_str(ending);
        }
        updated.push_str(line);
    }
    if !replacement_lines.is_empty() {
        updated.push_str(ending);
    }
    for part in &parts[(request.end_line as usize)..] {
        updated.push_str(part);
    }

    let new_bytes = updated.into_bytes();
    let new_sha = short_hash(&new_bytes);
    let diff_preview = build_preview(
        &parts[(request.start_line as usize - 1)..(request.end_line as usize)],
        &request.replacement,
    );

    if !request.dry_run {
        write_atomic(&abs, &new_bytes)?;
    }

    Ok(PatchOutcome {
        path: request.path.clone(),
        previous_sha,
        new_sha,
        lines_before: total,
        lines_after: new_bytes.iter().filter(|b| **b == b'\n').count() as u32,
        replaced_lines: request.end_line - request.start_line + 1,
        dry_run: request.dry_run,
        diff_preview,
    })
}

fn short_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..16].to_string()
}

fn build_preview(removed: &[&str], replacement: &str) -> String {
    let added: Vec<&str> = replacement.split('\n').collect();
    let mut out = String::new();
    for line in removed.iter().take(6) {
        out.push_str("- ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if removed.len() > 6 {
        out.push_str(&format!("- … {} more lines\n", removed.len() - 6));
    }
    for line in added.iter().take(6) {
        out.push_str("+ ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if added.len() > 6 {
        out.push_str(&format!("+ … {} more lines\n", added.len() - 6));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-patch-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sample.txt"), "primeira\nsegunda\nterceira\n").unwrap();
        dir
    }

    fn request(start: u32, end: u32, replacement: &str) -> PatchRequest {
        PatchRequest {
            path: "sample.txt".into(),
            start_line: start,
            end_line: end,
            replacement: replacement.to_string(),
            expected_sha: String::new(),
            dry_run: false,
            allow_whole_file: false,
        }
    }

    fn sha_of(root: &std::path::Path) -> String {
        short_hash(&std::fs::read(root.join("sample.txt")).unwrap())
    }

    #[test]
    fn a_dry_run_does_not_write() {
        let root = workspace("dry");
        let mut req = request(2, 2, "alterada");
        req.dry_run = true;
        req.expected_sha = sha_of(&root);
        let outcome = apply_patch(&root, &req, None).unwrap();
        assert!(outcome.dry_run);
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_requested_range_is_replaced() {
        let root = workspace("apply");
        let mut req = request(2, 2, "alterada");
        req.expected_sha = sha_of(&root);
        apply_patch(&root, &req, None).unwrap();
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nalterada\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_patch_without_a_revision_is_refused() {
        let root = workspace("no-sha");
        let outcome = apply_patch(&root, &request(2, 2, "alterada"), None);
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_CONTENT_MISMATCH");
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stale_revision_is_rejected() {
        let root = workspace("stale");
        let mut req = request(2, 2, "alterada");
        req.expected_sha = "0000000000000000".into();
        let outcome = apply_patch(&root, &req, None);
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_STALE_REVISION");
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_range_outside_the_file_is_rejected() {
        let root = workspace("range");
        let mut req = request(50, 60, "x");
        req.expected_sha = sha_of(&root);
        let outcome = apply_patch(&root, &req, None);
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_AMBIGUOUS_TARGET");
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_range_wider_than_the_read_excerpt_is_rejected() {
        let root = workspace("bound");
        let sha = sha_of(&root);
        let bound = ReadBound {
            start_line: 1,
            end_line: 2,
            sha: sha.clone(),
        };
        let mut req = request(1, 3, "x");
        req.expected_sha = sha;
        let outcome = apply_patch(&root, &req, Some(&bound));
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_AMBIGUOUS_TARGET");
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_whole_file_is_not_replaced_without_authorization() {
        let root = workspace("whole");
        let mut req = request(1, 3, "conteudo novo\n");
        req.expected_sha = sha_of(&root);
        let outcome = apply_patch(&root, &req, None);
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_AMBIGUOUS_TARGET");
        let content = std::fs::read_to_string(root.join("sample.txt")).unwrap();
        assert_eq!(content, "primeira\nsegunda\nterceira\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_line_endings_are_preserved() {
        let root = std::env::temp_dir().join("devzymer-patch-crlf");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("crlf.txt"), "a\r\nb\r\nc\r\n").unwrap();
        let sha = short_hash(&std::fs::read(root.join("crlf.txt")).unwrap());
        let outcome = apply_patch(
            &root,
            &PatchRequest {
                path: "crlf.txt".into(),
                start_line: 2,
                end_line: 2,
                replacement: "B".into(),
                expected_sha: sha,
                dry_run: false,
                allow_whole_file: false,
            },
            None,
        )
        .unwrap();
        assert!(!outcome.dry_run);
        let bytes = std::fs::read(root.join("crlf.txt")).unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes), "a\r\nB\r\nc\r\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_ascii_content_is_preserved_outside_the_range() {
        let root = std::env::temp_dir().join("devzymer-patch-utf8");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("u.txt"), "ação\nsegunda\n").unwrap();
        let sha = short_hash(&std::fs::read(root.join("u.txt")).unwrap());
        apply_patch(
            &root,
            &PatchRequest {
                path: "u.txt".into(),
                start_line: 2,
                end_line: 2,
                replacement: "alterada".into(),
                expected_sha: sha,
                dry_run: false,
                allow_whole_file: false,
            },
            None,
        )
        .unwrap();
        let bytes = std::fs::read(root.join("u.txt")).unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes), "ação\nalterada\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_outside_the_workspace_is_rejected() {
        let root = workspace("escape");
        let mut req = request(1, 1, "x");
        req.path = "../fora.txt".into();
        req.expected_sha = sha_of(&root);
        let outcome = apply_patch(&root, &req, None);
        assert!(outcome.is_err());
        assert_eq!(outcome.unwrap_err().code(), "PATCH_OUTSIDE_WORKSPACE");
        let _ = std::fs::remove_dir_all(&root);
    }
}
