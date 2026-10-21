use std::collections::HashSet;

use tree_sitter::Node;

use devzymer_core::model::{Language, Span, Symbol, SymbolId, SymbolKind};

const MAX_BODY_CHARS: usize = 60_000;
const MAX_SIGNATURE_CHARS: usize = 400;
const MAX_DOC_CHARS: usize = 400;

pub struct Extractor<'a> {
    path: &'a str,
    language: Language,
    #[allow(dead_code)]
    source: &'a str,
    bytes: &'a [u8],
    out: Vec<Symbol>,
    containers: Vec<String>,
    used_ids: HashSet<String>,
}

impl<'a> Extractor<'a> {
    pub fn new(path: &'a str, language: Language, source: &'a str) -> Self {
        Self {
            path,
            language,
            source,
            bytes: source.as_bytes(),
            out: Vec::new(),
            containers: Vec::new(),
            used_ids: HashSet::new(),
        }
    }

    pub fn finish(mut self) -> Vec<Symbol> {
        self.out
            .sort_by_key(|s| (s.span.start_byte, s.span.end_byte));
        self.out
    }

    pub fn visit(&mut self, node: Node) {
        let kind = node.kind();

        match kind {
            "decorated_definition" => {
                let mut decorators = Vec::new();
                for i in 0..node.child_count() {
                    let Some(child) = node.child(i) else {
                        continue;
                    };
                    if child.kind() == "decorator" {
                        decorators.push(self.text(child).trim().to_string());
                    }
                }
                for i in 0..node.child_count() {
                    let Some(child) = node.child(i) else {
                        continue;
                    };
                    if is_definition_kind(self.language, child.kind()) {
                        self.visit_with_decorators(child, &decorators);
                    }
                }
                return;
            }
            "import_statement" | "import_from_statement" | "use_declaration" => {
                self.emit_import(node, kind);
                return;
            }
            "call_expression" => {
                if self.emit_require(node) {
                    return;
                }
            }
            "export_statement" => {
                self.emit_export(node);
                self.visit_children(node);
                return;
            }
            "impl_item" => {
                let container = node
                    .child_by_field_name("type")
                    .map(|n| self.text(n).trim().to_string())
                    .unwrap_or_default();
                if !container.is_empty() {
                    self.containers.push(container);
                }
                self.visit_children(node);
                self.containers.pop();
                return;
            }
            "mod_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).trim().to_string();
                    if node.child_by_field_name("body").is_none() {
                        self.push_symbol(
                            node,
                            &name,
                            SymbolKind::Module,
                            Some(self.text(node).trim().to_string()),
                            None,
                            false,
                        );
                        return;
                    }
                }
            }
            _ => {}
        }

        if is_definition_kind(self.language, kind) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = self.text(name_node).trim().to_string();
                if !name.is_empty() && !is_pattern(name_node.kind()) && !is_trivial_name(&name) {
                    self.emit_definition(node, &name, &[]);
                    return;
                }
            }
        }

        self.visit_children(node);
    }

    fn visit_children(&mut self, node: Node) {
        let count = node.child_count();
        for i in 0..count {
            let Some(child) = node.child(i) else {
                continue;
            };
            self.visit(child);
        }
    }

    fn visit_with_decorators(&mut self, node: Node, decorators: &[String]) {
        if is_definition_kind(self.language, node.kind()) {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = self.text(name_node).trim().to_string();
                if !name.is_empty() {
                    self.emit_definition(node, &name, decorators);
                    return;
                }
            }
        }
        self.visit_children(node);
    }

    fn emit_definition(&mut self, node: Node, name: &str, decorators: &[String]) {
        let sym_kind = classify(self.language, node.kind());
        let signature = self.signature(node, decorators);
        let doc = self.doc(node);
        let exported = self.is_exported(node);
        self.push_symbol(node, name, sym_kind, signature, doc, exported);
        self.containers.push(name.to_string());
        self.visit_children(node);
        self.containers.pop();
    }

    fn emit_import(&mut self, node: Node, kind: &str) {
        let specifier = match kind {
            "import_statement" => node
                .child_by_field_name("source")
                .map(|n| {
                    self.text(n)
                        .trim()
                        .trim_matches(['"', '\'', '`'])
                        .to_string()
                })
                .unwrap_or_default(),
            "import_from_statement" => node
                .child_by_field_name("module_name")
                .map(|n| self.text(n).trim().to_string())
                .unwrap_or_default(),
            "use_declaration" => {
                let text = self.text(node).trim();
                text.strip_prefix("use ")
                    .map(|s| s.trim_end_matches(';').trim().to_string())
                    .unwrap_or_default()
            }
            _ => String::new(),
        };
        if specifier.is_empty() {
            return;
        }
        self.emit_import_symbol(node, specifier);
    }

    /// Extracts CommonJS `require('./module')` calls as import edges.
    fn emit_require(&mut self, node: Node) -> bool {
        let Some(function) = node.child_by_field_name("function") else {
            return false;
        };
        if function.kind() != "identifier" || self.text(function).trim() != "require" {
            return false;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return false;
        };
        let Some(first) = arguments.named_child(0) else {
            return false;
        };
        if first.kind() != "string" {
            return false;
        }
        let specifier = self
            .text(first)
            .trim()
            .trim_matches(['"', '\'', '`'])
            .to_string();
        if specifier.is_empty() {
            return false;
        }
        self.emit_import_symbol(node, specifier);
        true
    }

    fn emit_import_symbol(&mut self, node: Node, specifier: String) {
        let statement = collapse(self.text(node).trim());
        let span = self.span(node);
        let qualified = format!("import:{specifier}");
        let id = self.unique_id(&qualified);
        self.out.push(Symbol {
            id,
            name: specifier.clone(),
            qualified_name: qualified,
            kind: SymbolKind::Import,
            container: None,
            signature: Some(statement),
            doc: None,
            path: self.path.to_string(),
            language: self.language,
            span,
            exported: false,
            is_async: false,
            body: Some(self.text(node).trim().to_string()),
        });
    }

    fn emit_export(&mut self, node: Node) {
        let text = collapse(self.text(node).trim());
        let name = export_name(&text);
        if name.is_empty() {
            return;
        }
        let span = self.span(node);
        let qualified = format!("export:{name}");
        let id = self.unique_id(&qualified);
        self.out.push(Symbol {
            id,
            name,
            qualified_name: qualified,
            kind: SymbolKind::Export,
            container: None,
            signature: Some(text.clone()),
            doc: None,
            path: self.path.to_string(),
            language: self.language,
            span,
            exported: true,
            is_async: false,
            body: Some(text),
        });
    }

    fn push_symbol(
        &mut self,
        node: Node,
        name: &str,
        kind: SymbolKind,
        signature: Option<String>,
        doc: Option<String>,
        exported: bool,
    ) {
        let span = self.span(node);
        let qualified = if self.containers.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.containers.join("."), name)
        };
        let id = self.unique_id(&qualified);
        let body = self.body_text(node);
        let is_async = self.is_async(node);
        self.out.push(Symbol {
            id,
            name: name.to_string(),
            qualified_name: qualified,
            kind,
            container: self.containers.last().cloned(),
            signature,
            doc,
            path: self.path.to_string(),
            language: self.language,
            span,
            exported,
            is_async,
            body,
        });
    }

    fn unique_id(&mut self, qualified: &str) -> SymbolId {
        let mut candidate = SymbolId::new(self.path, qualified);
        let mut counter = 1u32;
        while self.used_ids.contains(candidate.as_str()) {
            counter += 1;
            candidate = SymbolId::new(self.path, &format!("{qualified}#{counter}"));
        }
        self.used_ids.insert(candidate.0.clone());
        candidate
    }

    fn text(&self, node: Node) -> &'a str {
        self.text_range(node.start_byte(), node.end_byte())
    }

    fn text_range(&self, start: usize, end: usize) -> &'a str {
        let start = start.min(self.bytes.len());
        let end = end.min(self.bytes.len());
        if start > end {
            return "";
        }
        std::str::from_utf8(&self.bytes[start..end]).unwrap_or("")
    }

    fn span(&self, node: Node) -> Span {
        Span::new(
            node.start_byte(),
            node.end_byte(),
            node.start_position().row as u32 + 1,
            node.end_position().row as u32 + 1,
        )
    }

    fn body_text(&self, node: Node) -> Option<String> {
        let text = self.text(node).trim();
        if text.is_empty() {
            return None;
        }
        Some(text.chars().take(MAX_BODY_CHARS).collect())
    }

    fn signature(&self, node: Node, decorators: &[String]) -> Option<String> {
        let body = node.child_by_field_name("body");
        let header = match body {
            Some(b) => self.text_range(node.start_byte(), b.start_byte()).trim(),
            None => self.text(node).trim(),
        };
        let mut parts: Vec<String> = Vec::new();
        for d in decorators {
            parts.push(d.clone());
        }
        parts.push(collapse(header));
        let joined = parts.join(" ").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined.chars().take(MAX_SIGNATURE_CHARS).collect())
        }
    }

    fn doc(&self, node: Node) -> Option<String> {
        // A comment directly above the definition, or directly above the
        // `export` wrapper that owns it, documents it. Named siblings are
        // used on purpose: the anonymous `export` keyword is the previous
        // token of the wrapped declaration, not a comment. The climb stops
        // at the first non wrapper, or when a comment does not sit right
        // above: another statement between them was not meant for it.
        let mut cursor = node;
        loop {
            let Some(prev) = cursor.prev_named_sibling() else {
                let parent = match cursor.parent() {
                    Some(p) => p,
                    None => break,
                };
                if !parent.kind().contains("export") {
                    break;
                }
                cursor = parent;
                continue;
            };
            if prev.kind().contains("comment") {
                let cleaned = clean_comment(self.text(prev));
                if !cleaned.is_empty() {
                    return Some(cleaned.chars().take(MAX_DOC_CHARS).collect());
                }
            }
            break;
        }
        if let Some(body) = node.child_by_field_name("body") {
            let count = body.named_child_count();
            if count > 0 {
                if let Some(first) = body.named_child(0) {
                    let kind = first.kind();
                    if kind == "string" || kind == "expression_statement" {
                        let text = self.text(first);
                        let cleaned = text
                            .trim()
                            .trim_start_matches('r')
                            .trim_start_matches('b')
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .trim();
                        if !cleaned.is_empty() {
                            return Some(cleaned.chars().take(MAX_DOC_CHARS).collect());
                        }
                    }
                }
            }
        }
        None
    }

    fn is_exported(&self, node: Node) -> bool {
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "export_statement" {
                return true;
            }
            current = parent.parent();
        }
        let text = self.text(node);
        let trimmed = text.trim_start();
        trimmed.starts_with("export ") || trimmed.starts_with("pub ")
    }

    fn is_async(&self, node: Node) -> bool {
        let header = match node.child_by_field_name("body") {
            Some(b) => self.text_range(node.start_byte(), b.start_byte()),
            None => self.text(node),
        };
        header.contains("async fn") || header.trim_start().starts_with("async ")
    }
}

fn export_name(text: &str) -> String {
    let rest = text
        .trim_start()
        .strip_prefix("export")
        .unwrap_or("")
        .trim_start();

    if rest.starts_with('{') {
        if let Some(end) = rest.find('}') {
            return rest[1..end]
                .split(',')
                .map(|part| part.split_whitespace().next().unwrap_or(""))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(",");
        }
    }

    let mut words = rest.split_whitespace();
    let mut first = words.next().unwrap_or("");

    if first == "default" {
        let mut rest_words = words;
        let mut keyword = rest_words.next().unwrap_or("");
        if keyword == "async" {
            keyword = rest_words.next().unwrap_or("");
        }
        if keyword == "function" || keyword == "class" {
            if let Some(name) = rest_words.next() {
                let name = identifier(name);
                if !name.is_empty() {
                    return name;
                }
            }
        }
        return "default".to_string();
    }

    if first == "async" {
        first = words.next().unwrap_or("");
    }
    if first == "function" {
        if let Some(star) = words.next() {
            let candidate = identifier(star.trim_start_matches('*'));
            if !candidate.is_empty() {
                return candidate;
            }
        }
        return String::new();
    }
    if matches!(
        first,
        "class" | "const" | "let" | "var" | "interface" | "type" | "enum" | "abstract"
    ) {
        if let Some(candidate) = words.next() {
            let candidate = identifier(candidate.trim_start_matches('*'));
            if !candidate.is_empty() {
                return candidate;
            }
        }
    }
    String::new()
}

/// Leading identifier of a declaration, without generics or parameters.
fn identifier(text: &str) -> String {
    text.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect()
}

/// Destructuring patterns do not describe a single retrievable symbol.
fn is_pattern(kind: &str) -> bool {
    matches!(kind, "object_pattern" | "array_pattern" | "rest_pattern")
}

/// Single letter names carry no retrieval value and only add noise.
fn is_trivial_name(name: &str) -> bool {
    name.chars().count() < 2
}

fn clean_comment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let l = line.trim();
        let l = l
            .trim_start_matches("/**")
            .trim_start_matches("///")
            .trim_start_matches("//")
            .trim_start_matches("/*")
            .trim_start_matches('*')
            .trim_end_matches("*/")
            .trim();
        if !l.is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(l);
        }
    }
    out
}

pub fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

pub fn is_definition_kind(language: Language, kind: &str) -> bool {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript | Language::Jsx => matches!(
            kind,
            "function_declaration"
                | "generator_function_declaration"
                | "class_declaration"
                | "abstract_class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "type_alias_declaration"
                | "method_definition"
                | "method_signature"
                | "public_field_definition"
                | "field_definition"
                | "function_signature"
                | "abstract_method_signature"
                | "module"
                | "internal_module"
                | "variable_declarator"
        ),
        Language::Python => matches!(
            kind,
            "function_definition" | "class_definition" | "decorated_definition"
        ),
        Language::Rust => matches!(
            kind,
            "function_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "mod_item"
                | "const_item"
                | "static_item"
                | "type_item"
                | "macro_definition"
                | "union_item"
        ),
        _ => false,
    }
}

pub fn classify(language: Language, kind: &str) -> SymbolKind {
    match language {
        Language::Rust => match kind {
            "function_item" => SymbolKind::Function,
            "struct_item" | "union_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "mod_item" => SymbolKind::Module,
            "const_item" | "static_item" => SymbolKind::Constant,
            "type_item" => SymbolKind::TypeAlias,
            "macro_definition" => SymbolKind::Macro,
            _ => SymbolKind::Unknown,
        },
        Language::Python => match kind {
            "function_definition" => SymbolKind::Function,
            "class_definition" => SymbolKind::Class,
            _ => SymbolKind::Unknown,
        },
        _ => match kind {
            "function_declaration" | "generator_function_declaration" | "function_signature" => {
                SymbolKind::Function
            }
            "class_declaration" | "abstract_class_declaration" => SymbolKind::Class,
            "interface_declaration" => SymbolKind::Interface,
            "enum_declaration" => SymbolKind::Enum,
            "type_alias_declaration" => SymbolKind::TypeAlias,
            "method_definition" | "method_signature" | "abstract_method_signature" => {
                SymbolKind::Method
            }
            "public_field_definition" | "field_definition" => SymbolKind::Field,
            "module" | "internal_module" => SymbolKind::Module,
            "variable_declarator" => SymbolKind::Variable,
            _ => SymbolKind::Unknown,
        },
    }
}
