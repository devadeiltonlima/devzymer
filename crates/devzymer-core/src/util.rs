use std::path::{Component, Path, PathBuf};

/// Converts a path to the canonical relative, forward-slash form used as the
/// primary key for files in the index.
pub fn relative_slash(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(slash(rel))
}

pub fn slash(path: &Path) -> String {
    let mut out = String::with_capacity(path.as_os_str().len());
    for part in path.components() {
        match part {
            Component::Normal(s) => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(&s.to_string_lossy());
            }
            Component::CurDir => {}
            other => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(&other.as_os_str().to_string_lossy());
            }
        }
    }
    out
}

/// Resolves a user supplied path against the workspace root and refuses to
/// escape it, including through symlinks.
pub fn resolve_inside(root: &Path, candidate: &Path) -> crate::Result<PathBuf> {
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = normalize(&joined);
    if !normalized.starts_with(root) {
        return Err(crate::Error::PathEscape(normalized.display().to_string()));
    }
    if let Ok(canonical_root) = root.canonicalize() {
        if let Ok(canonical) = normalized.canonicalize() {
            if !canonical.starts_with(&canonical_root) {
                return Err(crate::Error::PathEscape(canonical.display().to_string()));
            }
        }
    }
    Ok(normalized)
}

/// Lexical path normalization: resolves `.` and `..` without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Rough token estimate for a text body. Calibrated against the o200k family:
/// roughly four characters per token for source code.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    let by_chars = chars.div_ceil(4);
    let by_words = words * 5 / 4 + words / 8;
    by_chars.max(by_words)
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Truncates a string to `max` characters, appending an ellipsis marker.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// First line of a doc comment, normalized to a single line.
pub fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Detects likely secret material so it can be redacted from logs and caches.
pub fn looks_secret(text: &str) -> bool {
    const MARKERS: [&str; 10] = [
        "AKIA",
        "ghp_",
        "gho_",
        "github_pat_",
        "sk-",
        "-----BEGIN",
        "xoxb-",
        "xoxp-",
        "AIza",
        "eyJhbGciOi",
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

/// Detects the conventional locations of dedicated test files. The rule is
/// shared by the graph builder and the permission aware hook so both agree on
/// what counts as a test file.
pub fn is_test_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file = normalized.rsplit('/').next().unwrap_or(&normalized);
    if file.contains(".test.")
        || file.contains(".spec.")
        || file.contains("_test.")
        || file.contains("_spec.")
        || file.contains("test_.")
    {
        return true;
    }
    if file.starts_with("test_") || file.starts_with("spec_") {
        return true;
    }
    if file.ends_with("test.java")
        || file.ends_with("test.go")
        || file.ends_with("tests.rs")
        || file.ends_with("test.ts")
        || file.ends_with("test.py")
    {
        return true;
    }
    normalized
        .split('/')
        .any(|part| matches!(part, "tests" | "test" | "__tests__" | "spec" | "specs"))
}

/// Removes the Windows extended length prefix (`\\?\`) that `canonicalize`
/// adds, keeping the paths readable in configuration files and output.
pub fn strip_extended_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// Canonicalizes a path and removes the Windows extended length prefix.
pub fn canonical_display(path: &Path) -> PathBuf {
    match path.canonicalize() {
        Ok(resolved) => strip_extended_prefix(&resolved),
        Err(_) => path.to_path_buf(),
    }
}

/// Splits an identifier into its lowercase words: `useVoiceCall` becomes
/// `use voice call` and `API_GATEWAY_TOKEN` becomes `api gateway token`.
/// Words are separated by casing boundaries, digits and separators.
pub fn split_identifier(value: &str) -> String {
    #[derive(PartialEq)]
    enum CharKind {
        Upper,
        Lower,
        Digit,
        Other,
    }
    fn kind_of(c: char) -> CharKind {
        if c.is_ascii_uppercase() {
            CharKind::Upper
        } else if c.is_ascii_lowercase() {
            CharKind::Lower
        } else if c.is_ascii_digit() {
            CharKind::Digit
        } else {
            CharKind::Other
        }
    }
    let mut out = String::with_capacity(value.len() + 8);
    let mut prev: CharKind = CharKind::Other;
    let mut word_open = false;
    for (index, ch) in value.char_indices() {
        let kind = kind_of(ch);
        match kind {
            CharKind::Other => {
                if word_open {
                    out.push(' ');
                    word_open = false;
                }
                prev = kind;
                continue;
            }
            CharKind::Upper => {
                let previous_is_upper = prev == CharKind::Upper;
                let next_is_lower = value[index + ch.len_utf8()..]
                    .chars()
                    .next()
                    .map(|n| n.is_ascii_lowercase())
                    .unwrap_or(false);
                let boundary = (word_open && !previous_is_upper)
                    || (word_open && previous_is_upper && next_is_lower);
                if boundary {
                    out.push(' ');
                    word_open = false;
                }
            }
            CharKind::Digit => {
                if word_open && prev != CharKind::Digit {
                    out.push(' ');
                    word_open = false;
                }
            }
            CharKind::Lower => {}
        }
        if !word_open && index > 0 && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
        word_open = true;
        prev = kind;
    }
    out.split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replaces accented letters with their base letter, so Portuguese and
/// English text compare equal regardless of diacritics.
pub fn diacritic_fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            'á' | 'à' | 'â' | 'ã' | 'ä' => out.push('a'),
            'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => out.push('A'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'É' | 'È' | 'Ê' | 'Ë' => out.push('E'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'Í' | 'Ì' | 'Î' | 'Ï' => out.push('I'),
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => out.push('o'),
            'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => out.push('O'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'Ú' | 'Ù' | 'Û' | 'Ü' => out.push('U'),
            'ç' | 'Ç' => out.push('c'),
            'ñ' | 'Ñ' => out.push('n'),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_inside_the_workspace_are_accepted() {
        let root = std::env::temp_dir().join("devzymer-util-test");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let resolved = resolve_inside(&root, Path::new("src/main.rs")).unwrap();
        assert!(resolved.ends_with("src/main.rs"));
    }

    #[test]
    fn paths_that_escape_the_workspace_are_rejected() {
        let root = std::env::temp_dir().join("devzymer-util-test");
        std::fs::create_dir_all(&root).unwrap();
        let outcome = resolve_inside(&root, Path::new("../outside.txt"));
        assert!(outcome.is_err());
    }

    #[test]
    fn normalize_resolves_parent_segments() {
        let normalized = normalize(Path::new("a/b/../c/./d"));
        assert_eq!(normalized, PathBuf::from("a/c/d"));
    }

    #[test]
    fn token_estimate_grows_with_the_text() {
        let short = estimate_tokens("fn main() {}");
        let long = estimate_tokens(&"a".repeat(4000));
        assert!(short > 0);
        assert!(long > short * 10);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn secret_markers_are_detected() {
        assert!(looks_secret("AKIAIOSFODNN7EXAMPLE"));
        assert!(looks_secret("token ghp_1234567890abcdef"));
        assert!(!looks_secret("const value = 42;"));
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let rel =
            relative_slash(Path::new("/repo"), Path::new("/repo/src/auth/session.ts")).unwrap();
        assert_eq!(rel, "src/auth/session.ts");
    }

    #[test]
    fn test_locations_are_recognised() {
        assert!(is_test_path("src/foo.test.ts"));
        assert!(is_test_path("src/__tests__/foo.ts"));
        assert!(is_test_path("tests/test_foo.py"));
        assert!(is_test_path("src/foo_spec.rs"));
        assert!(is_test_path("src\\foo.test.ts"));
        assert!(!is_test_path("src/foo.ts"));
        assert!(!is_test_path("src/testing.ts"));
    }

    #[test]
    fn identifiers_are_split_into_their_words() {
        assert_eq!(split_identifier("useVoiceCall"), "use voice call");
        assert_eq!(split_identifier("API_GATEWAY_TOKEN"), "api gateway token");
        assert_eq!(split_identifier("VoiceCall"), "voice call");
        assert_eq!(split_identifier("HTTPServer"), "http server");
        assert_eq!(
            split_identifier("limitadorDeChamadas2"),
            "limitador de chamadas 2"
        );
        assert_eq!(split_identifier(""), "");
    }

    #[test]
    fn accents_fold_for_comparison() {
        assert_eq!(diacritic_fold("ação"), "acao");
        assert_eq!(diacritic_fold("TRANSFERÊNCIA"), "TRANSFERENCIA");
        assert_eq!(diacritic_fold("atende"), "atende");
    }
}
