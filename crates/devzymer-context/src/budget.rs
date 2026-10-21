use serde::Serialize;

use devzymer_core::util::estimate_tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextLevel {
    /// Path, symbol name and line range only.
    Location = 1,
    /// Adds signatures and short documentation.
    Evidence = 2,
    /// Adds the complete implementation of the symbol.
    Implementation = 3,
    /// Adds directly related definitions.
    Dependency = 4,
    /// Adds broader file level context.
    Expanded = 5,
}

impl ContextLevel {
    pub fn from_u8(value: u8) -> ContextLevel {
        match value {
            1 => ContextLevel::Location,
            2 => ContextLevel::Evidence,
            3 => ContextLevel::Implementation,
            4 => ContextLevel::Dependency,
            5 => ContextLevel::Expanded,
            _ => ContextLevel::Evidence,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn display(self) -> &'static str {
        match self {
            ContextLevel::Location => "location",
            ContextLevel::Evidence => "evidence",
            ContextLevel::Implementation => "implementation",
            ContextLevel::Dependency => "dependency",
            ContextLevel::Expanded => "expanded",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Fragment {
    pub path: String,
    pub symbol: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub confidence: String,
    pub score: f64,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub text: Option<String>,
    pub tokens: usize,
}

impl Fragment {
    pub fn location_line(&self) -> String {
        format!(
            "{}:{}-{} | {} | {}",
            self.path, self.start_line, self.end_line, self.symbol, self.confidence
        )
    }
}

/// Token estimate of a fragment, considering the fields it actually carries.
pub fn fragment_tokens(fragment: &Fragment) -> usize {
    let mut total = estimate_tokens(&format!(
        "{}:{} {}",
        fragment.path, fragment.start_line, fragment.symbol
    ));
    if let Some(sig) = &fragment.signature {
        total += estimate_tokens(sig);
    }
    if let Some(doc) = &fragment.doc {
        total += estimate_tokens(doc);
    }
    if let Some(text) = &fragment.text {
        total += estimate_tokens(text);
    }
    total
}
