use tree_sitter::Language as TsLanguage;
use tree_sitter::Parser;

use devzymer_core::model::Language;

/// Resolves the compiled tree-sitter grammar for a supported language.
pub fn language_for(language: Language) -> Option<TsLanguage> {
    match language {
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Tsx => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Language::JavaScript | Language::Jsx => Some(tree_sitter_javascript::LANGUAGE.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        _ => None,
    }
}

/// Languages that have a tree-sitter grammar compiled into the binary.
pub fn supported_languages() -> Vec<Language> {
    vec![
        Language::TypeScript,
        Language::Tsx,
        Language::JavaScript,
        Language::Jsx,
        Language::Python,
        Language::Rust,
    ]
}

pub struct ParserRegistry;

impl ParserRegistry {
    pub fn new() -> Self {
        ParserRegistry
    }

    pub fn parser_for(language: Language) -> Option<Parser> {
        let grammar = language_for(language)?;
        let mut parser = Parser::new();
        parser.set_language(&grammar).ok()?;
        Some(parser)
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extracts the symbols of a source file, falling back to heuristic indexing
/// for languages without a compiled grammar.
pub fn extract_symbols(
    path: &str,
    language: Language,
    source: &str,
) -> Vec<devzymer_core::model::Symbol> {
    match language_for(language) {
        Some(grammar) => {
            let mut parser = Parser::new();
            if parser.set_language(&grammar).is_err() {
                return crate::fallback::extract_symbols(path, language, source);
            }
            let tree = match parser.parse(source, None) {
                Some(t) => t,
                None => return crate::fallback::extract_symbols(path, language, source),
            };
            let mut extractor = crate::parse::extract::Extractor::new(path, language, source);
            extractor.visit(tree.root_node());
            extractor.finish()
        }
        None => crate::fallback::extract_symbols(path, language, source),
    }
}

pub mod extract;
#[cfg(test)]
mod parse_tests;
