use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// Byte and line extent of a construct inside a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: u32,
    pub end_line: u32,
}

impl Span {
    pub fn new(start_byte: usize, end_byte: usize, start_line: u32, end_line: u32) -> Self {
        Self {
            start_byte,
            end_byte,
            start_line,
            end_line,
        }
    }

    pub fn contains_byte(&self, offset: usize) -> bool {
        offset >= self.start_byte && offset < self.end_byte
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Rust,
    Go,
    Java,
    Kotlin,
    C,
    Cpp,
    CSharp,
    Ruby,
    Php,
    Swift,
    Sql,
    Unknown,
}

impl Language {
    pub fn from_path(path: &Path) -> Language {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "ts" | "mts" | "cts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "js" | "mjs" | "cjs" => Language::JavaScript,
            "jsx" => Language::Jsx,
            "py" | "pyi" | "pyw" => Language::Python,
            "rs" => Language::Rust,
            "go" => Language::Go,
            "java" => Language::Java,
            "kt" | "kts" => Language::Kotlin,
            "c" | "h" => Language::C,
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Language::Cpp,
            "cs" => Language::CSharp,
            "rb" => Language::Ruby,
            "php" => Language::Php,
            "swift" => Language::Swift,
            "sql" => Language::Sql,
            _ => Language::Unknown,
        }
    }

    /// True when a dedicated tree-sitter grammar is compiled in.
    pub fn has_parser(self) -> bool {
        matches!(
            self,
            Language::TypeScript
                | Language::Tsx
                | Language::JavaScript
                | Language::Jsx
                | Language::Python
                | Language::Rust
        )
    }

    pub fn display(self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Jsx => "jsx",
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::CSharp => "csharp",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Swift => "swift",
            Language::Sql => "sql",
            Language::Unknown => "unknown",
        }
    }

    pub fn from_display(value: &str) -> Option<Language> {
        let v = value.to_ascii_lowercase();
        Some(match v.as_str() {
            "typescript" | "ts" => Language::TypeScript,
            "tsx" => Language::Tsx,
            "javascript" | "js" => Language::JavaScript,
            "jsx" => Language::Jsx,
            "python" | "py" => Language::Python,
            "rust" | "rs" => Language::Rust,
            "go" | "golang" => Language::Go,
            "java" => Language::Java,
            "kotlin" | "kt" => Language::Kotlin,
            "c" => Language::C,
            "cpp" | "c++" => Language::Cpp,
            "csharp" | "c#" | "cs" => Language::CSharp,
            "ruby" | "rb" => Language::Ruby,
            "php" => Language::Php,
            "swift" => Language::Swift,
            "sql" => Language::Sql,
            _ => return None,
        })
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Constructor,
    Class,
    Interface,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Constant,
    Variable,
    Field,
    Property,
    Module,
    Macro,
    Import,
    Export,
    Test,
    Unknown,
}

impl SymbolKind {
    pub fn display(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::Field => "field",
            SymbolKind::Property => "property",
            SymbolKind::Module => "module",
            SymbolKind::Macro => "macro",
            SymbolKind::Import => "import",
            SymbolKind::Export => "export",
            SymbolKind::Test => "test",
            SymbolKind::Unknown => "unknown",
        }
    }

    pub fn from_display(value: &str) -> SymbolKind {
        match value {
            "function" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "constructor" => SymbolKind::Constructor,
            "class" => SymbolKind::Class,
            "interface" => SymbolKind::Interface,
            "struct" => SymbolKind::Struct,
            "enum" => SymbolKind::Enum,
            "trait" => SymbolKind::Trait,
            "type" => SymbolKind::TypeAlias,
            "constant" => SymbolKind::Constant,
            "variable" => SymbolKind::Variable,
            "field" => SymbolKind::Field,
            "property" => SymbolKind::Property,
            "module" => SymbolKind::Module,
            "macro" => SymbolKind::Macro,
            "import" => SymbolKind::Import,
            "export" => SymbolKind::Export,
            "test" => SymbolKind::Test,
            _ => SymbolKind::Unknown,
        }
    }
}

impl fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display())
    }
}

/// Stable identifier derived from the file path plus the qualified symbol name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SymbolId(pub String);

impl SymbolId {
    pub fn new(path: &str, qualified_name: &str) -> SymbolId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.as_bytes());
        hasher.update(b"\x00");
        hasher.update(qualified_name.as_bytes());
        SymbolId(hasher.finalize().to_hex()[..16].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub container: Option<String>,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub path: String,
    pub language: Language,
    pub span: Span,
    pub exported: bool,
    pub is_async: bool,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Defines,
    Imports,
    Exports,
    Calls,
    References,
    Implements,
    Extends,
    DependsOn,
    TestedBy,
}

impl RelationKind {
    pub fn display(self) -> &'static str {
        match self {
            RelationKind::Defines => "defines",
            RelationKind::Imports => "imports",
            RelationKind::Exports => "exports",
            RelationKind::Calls => "calls",
            RelationKind::References => "references",
            RelationKind::Implements => "implements",
            RelationKind::Extends => "extends",
            RelationKind::DependsOn => "depends_on",
            RelationKind::TestedBy => "tested_by",
        }
    }

    pub fn from_display(value: &str) -> RelationKind {
        match value {
            "defines" => RelationKind::Defines,
            "imports" => RelationKind::Imports,
            "exports" => RelationKind::Exports,
            "calls" => RelationKind::Calls,
            "references" => RelationKind::References,
            "implements" => RelationKind::Implements,
            "extends" => RelationKind::Extends,
            "depends_on" => RelationKind::DependsOn,
            "tested_by" => RelationKind::TestedBy,
            _ => RelationKind::References,
        }
    }

    /// Relations that describe a directed dependency from source to target.
    pub fn is_dependency(self) -> bool {
        matches!(
            self,
            RelationKind::Calls
                | RelationKind::Imports
                | RelationKind::DependsOn
                | RelationKind::Implements
                | RelationKind::Extends
        )
    }
}

/// Confidence of a graph edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Established by an unambiguous static rule.
    Confirmed,
    /// Established by name matching or textual heuristics.
    Heuristic,
    /// The edge is known to exist but its target could not be resolved.
    Unresolved,
}

impl Confidence {
    pub fn display(self) -> &'static str {
        match self {
            Confidence::Confirmed => "confirmed",
            Confidence::Heuristic => "heuristic",
            Confidence::Unresolved => "unresolved",
        }
    }

    pub fn from_display(value: &str) -> Confidence {
        match value {
            "confirmed" => Confidence::Confirmed,
            "heuristic" => Confidence::Heuristic,
            _ => Confidence::Unresolved,
        }
    }

    pub fn weight(self) -> f64 {
        match self {
            Confidence::Confirmed => 1.0,
            Confidence::Heuristic => 0.6,
            Confidence::Unresolved => 0.2,
        }
    }
}

/// Node of the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum Entity {
    Symbol(SymbolId),
    File(String),
    External(String),
}

impl Entity {
    pub fn key(&self) -> String {
        match self {
            Entity::Symbol(id) => format!("s:{id}"),
            Entity::File(path) => format!("f:{path}"),
            Entity::External(name) => format!("x:{name}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Entity> {
        let (prefix, rest) = key.split_once(':')?;
        match prefix {
            "s" => Some(Entity::Symbol(SymbolId(rest.to_string()))),
            "f" => Some(Entity::File(rest.to_string())),
            "x" => Some(Entity::External(rest.to_string())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub src: Entity,
    pub dst: Entity,
    pub kind: RelationKind,
    pub confidence: Confidence,
    pub evidence: Option<String>,
    pub file: Option<String>,
}
