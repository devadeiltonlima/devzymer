use std::path::Path;

/// Reads a repository file as text.
pub fn read_source(root: &Path, relative: &str) -> Option<String> {
    let path = devzymer_core::util::resolve_inside(root, Path::new(relative)).ok()?;
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The plain identifier of a qualified symbol name.
pub fn identifier_of(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified).trim()
}

/// True when the text contains the identifier as a whole word.
pub fn contains_identifier(text: &str, identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let target = identifier.as_bytes();
    let mut start = 0usize;
    while let Some(found) = text[start..].find(identifier) {
        let index = start + found;
        let before_ok = index == 0 || !is_word_byte(bytes[index - 1]);
        let after = index + target.len();
        let after_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = index + 1;
        if start >= text.len() {
            break;
        }
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// True when the given one based line contains the identifier.
pub fn line_contains(lines: &[&str], line: u32, identifier: &str) -> bool {
    if line == 0 {
        return false;
    }
    let index = line as usize - 1;
    lines
        .get(index)
        .map(|text| contains_identifier(text, identifier))
        .unwrap_or(false)
}

/// The documentation comment written above a line, if there is one.
pub fn doc_comment_above(lines: &[&str], line: u32) -> Option<String> {
    if line < 2 {
        return None;
    }
    let mut collected: Vec<String> = Vec::new();
    let mut index = line as usize - 2;
    while let Some(text) = lines.get(index) {
        let trimmed = text.trim();
        if trimmed.starts_with("//") {
            let content = trimmed
                .trim_start_matches("/")
                .trim_start_matches('/')
                .trim();
            collected.push(content.to_string());
        } else if trimmed.starts_with('*') && trimmed.ends_with('}') {
            collected.push(
                trimmed
                    .trim_matches(['*', '/', '{', '}'])
                    .trim()
                    .to_string(),
            );
        } else {
            break;
        }
        if index == 0 {
            break;
        }
        index -= 1;
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    let joined = collected.join(" ");
    let joined = joined.trim();
    if joined.is_empty() {
        None
    } else {
        Some(joined.to_string())
    }
}

/// Removes an identifier from a text, so a description cannot leak the answer.
pub fn redact_identifier(text: &str, identifier: &str) -> String {
    if identifier.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(found) = rest.find(identifier) {
        out.push_str(&rest[..found]);
        out.push_str("algo");
        rest = &rest[found + identifier.len()..];
    }
    out.push_str(rest);
    out
}

/// The first sentence of a text, limited to a maximum length.
pub fn first_sentence(text: &str, max_chars: usize) -> String {
    let cleaned = text.replace(['\n', '\r'], " ");
    let mut sentence = String::new();
    for part in cleaned.split(". ") {
        if sentence.is_empty() {
            sentence = part.trim().to_string();
        } else {
            sentence.push_str(". ");
            sentence.push_str(part.trim());
        }
        if sentence.chars().count() >= max_chars {
            break;
        }
    }
    if sentence.chars().count() > max_chars {
        sentence = sentence.chars().take(max_chars).collect();
    }
    sentence.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_matched_as_whole_words() {
        assert!(contains_identifier("const objeto = 1;", "objeto"));
        assert!(!contains_identifier("const objetoX = 1;", "objeto"));
        assert!(!contains_identifier("const xobjeto = 1;", "objeto"));
    }

    #[test]
    fn the_identifier_of_a_qualified_name_is_the_last_part() {
        assert_eq!(identifier_of("SampleService.ferramentas"), "ferramentas");
        assert_eq!(identifier_of("objeto"), "objeto");
    }

    #[test]
    fn the_documentation_above_a_line_is_collected() {
        let lines = vec!["// first line", "// second line", "function alvo() {}"];
        let doc = doc_comment_above(&lines, 3).expect("doc");
        assert!(doc.contains("first line"));
        assert!(doc.contains("second line"));
    }

    #[test]
    fn redaction_removes_the_answer() {
        let redacted = redact_identifier("chama validarSessao agora", "validarSessao");
        assert!(!redacted.contains("validarSessao"));
        assert!(redacted.contains("algo"));
    }
}
