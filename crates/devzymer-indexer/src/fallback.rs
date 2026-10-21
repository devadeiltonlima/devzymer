use std::collections::HashMap;

use devzymer_core::model::{Language, Span, Symbol, SymbolId, SymbolKind};

/// Heuristic symbol extraction for languages without a compiled tree-sitter
/// grammar. The output is explicitly marked as heuristic by the caller.
pub fn extract_symbols(path: &str, language: Language, source: &str) -> Vec<Symbol> {
    let patterns = patterns_for(language);
    if patterns.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<Symbol> = Vec::new();
    let mut used_ids: HashMap<String, u32> = HashMap::new();
    let mut byte_offset = 0usize;

    for (lineno, line) in source.lines().enumerate() {
        let line_start = byte_offset;
        byte_offset += line.len() + 1;
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        for (kind, regex) in &patterns {
            let Some(caps) = regex.captures(line) else {
                continue;
            };
            let Some(m) = caps.get(1) else { continue };
            let name = m.as_str().trim();
            if name.is_empty() || name.len() > 128 {
                continue;
            }
            let start = line_start + m.start();
            let end = line_start + m.end();
            let qualified = name.to_string();
            let mut candidate = SymbolId::new(path, &qualified);
            let counter = used_ids.entry(qualified.clone()).or_insert(0);
            *counter += 1;
            if *counter > 1 {
                candidate = SymbolId::new(path, &format!("{qualified}#{}", *counter));
            }
            out.push(Symbol {
                id: candidate,
                name: name.to_string(),
                qualified_name: qualified,
                kind: *kind,
                container: None,
                signature: Some(trimmed.chars().take(300).collect()),
                doc: None,
                path: path.to_string(),
                language,
                span: Span::new(start, end, lineno as u32 + 1, lineno as u32 + 1),
                exported: trimmed.starts_with("pub ") || trimmed.starts_with("public "),
                is_async: false,
                body: Some(trimmed.to_string()),
            });
            break;
        }
    }
    out
}

fn patterns_for(language: Language) -> Vec<(SymbolKind, regex::Regex)> {
    let raw: Vec<(SymbolKind, &str)> = match language {
        Language::Go => vec![
            (
                SymbolKind::Function,
                r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)",
            ),
            (SymbolKind::Struct, r"^type\s+([A-Za-z_]\w*)\s+struct\b"),
            (
                SymbolKind::Interface,
                r"^type\s+([A-Za-z_]\w*)\s+interface\b",
            ),
            (SymbolKind::Constant, r"^const\s+([A-Za-z_]\w*)"),
        ],
        Language::Java | Language::Kotlin => vec![
            (
                SymbolKind::Class,
                r"^\s*(?:public\s+|private\s+|protected\s+|internal\s+|abstract\s+|final\s+|sealed\s+|data\s+|open\s+)*(?:class|interface|enum|record|object)\s+([A-Za-z_]\w*)",
            ),
            (
                SymbolKind::Method,
                r"^\s*(?:public\s+|private\s+|protected\s+|internal\s+|static\s+|final\s+|abstract\s+|override\s+|suspend\s+|open\s+|fun\s+)*[\w<>\[\],.?\s]+\s+([A-Za-z_]\w*)\s*\([^;]*\)\s*(?:throws\s+[\w,.\s]+)?\{",
            ),
        ],
        Language::CSharp => vec![
            (
                SymbolKind::Class,
                r"^\s*(?:public\s+|private\s+|internal\s+|protected\s+|static\s+|sealed\s+|abstract\s+|partial\s+|record\s+)*(?:class|interface|enum|struct|record)\s+([A-Za-z_]\w*)",
            ),
            (
                SymbolKind::Method,
                r"^\s*(?:public\s+|private\s+|protected\s+|internal\s+|static\s+|virtual\s+|override\s+|async\s+|sealed\s+|partial\s+)+[\w<>\[\],.?\s]+\s+([A-Za-z_]\w*)\s*\([^;]*\)",
            ),
        ],
        Language::C | Language::Cpp => vec![
            (
                SymbolKind::Function,
                r"^\s*(?:static\s+|inline\s+|virtual\s+|extern\s+|const\s+)+[\w:*&<>,\s]+\s+([A-Za-z_]\w*)\s*\([^;]*\)\s*(?:const\s*)?\{",
            ),
            (
                SymbolKind::Struct,
                r"^\s*(?:typedef\s+)?struct\s+([A-Za-z_]\w*)",
            ),
            (SymbolKind::Class, r"^\s*class\s+([A-Za-z_]\w*)"),
        ],
        Language::Ruby => vec![
            (SymbolKind::Function, r"^\s*def\s+([A-Za-z_][\w?!.]*)"),
            (SymbolKind::Class, r"^\s*(?:class|module)\s+([A-Za-z_]\w*)"),
        ],
        Language::Php => vec![
            (
                SymbolKind::Function,
                r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|abstract\s+|final\s+)*function\s+([A-Za-z_]\w*)",
            ),
            (
                SymbolKind::Class,
                r"^\s*(?:abstract\s+|final\s+)*(?:class|interface|trait)\s+([A-Za-z_]\w*)",
            ),
        ],
        Language::Swift => vec![
            (
                SymbolKind::Function,
                r"^\s*(?:public\s+|private\s+|internal\s+|static\s+|mutating\s+|override\s+|final\s+)*func\s+([A-Za-z_]\w*)",
            ),
            (
                SymbolKind::Class,
                r"^\s*(?:public\s+|private\s+|internal\s+|open\s+|final\s+)*(?:class|struct|enum|protocol|extension)\s+([A-Za-z_]\w*)",
            ),
        ],
        _ => Vec::new(),
    };

    raw.into_iter()
        .filter_map(|(kind, pattern)| regex::Regex::new(pattern).ok().map(|r| (kind, r)))
        .collect()
}
