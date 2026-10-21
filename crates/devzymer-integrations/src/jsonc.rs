//! Editor de JSON com comentários.
//!
//! O OpenCode guarda a configuração global em `opencode.jsonc`, um JSON com
//! comentários. Reescrever o arquivo perderia as escolhas do usuário, então a
//! alteração é feita sobre o texto original: o documento é analisado com as
//! posições de cada valor, e só o trecho afetado é substituído.

use std::collections::BTreeMap;
use std::ops::Range;

/// Uma posição do documento, com o texto que começa nela.
#[derive(Debug, Clone)]
pub struct Node {
    pub path: Vec<String>,
    /// Intervalo do texto que contém o valor.
    pub span: Range<usize>,
    /// Valor analisado, para consulta.
    pub value: Json,
}

/// O subconjunto de JSON necessário para inspecionar e editar a configuração.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(map) => Some(map),
            _ => None,
        }
    }
}

/// Analisa um documento JSON com comentários e devolve os valores com posição.
pub fn parse(text: &str) -> Result<Vec<Node>, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        text,
        pos: 0,
        nodes: Vec::new(),
    };
    parser.skip_trivia();
    let value = parser.value(&mut Vec::new())?;
    parser.skip_trivia();
    if parser.pos != parser.bytes.len() {
        return Err(format!("conteúdo inesperado na posição {}", parser.pos));
    }
    let _ = value;
    Ok(parser.nodes)
}

/// Devolve o valor do documento, sem as posições.
pub fn value(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        text,
        pos: 0,
        nodes: Vec::new(),
    };
    parser.skip_trivia();
    let value = parser.value(&mut Vec::new())?;
    Ok(value)
}

/// Localiza o valor de um caminho, com a posição no texto original.
pub fn find(text: &str, path: &[&str]) -> Result<Option<Node>, String> {
    let nodes = parse(text)?;
    Ok(nodes
        .into_iter()
        .find(|node| node.path == path.iter().map(|s| s.to_string()).collect::<Vec<_>>()))
}

/// Substitui o valor de um caminho, preservando o restante do texto.
pub fn replace(text: &str, path: &[&str], new_text: &str) -> Result<String, String> {
    let Some(node) = find(text, path)? else {
        return Err(format!("caminho não encontrado: {}", path.join(".")));
    };
    let mut out = String::with_capacity(text.len() + new_text.len());
    out.push_str(&text[..node.span.start]);
    out.push_str(new_text);
    out.push_str(&text[node.span.end..]);
    Ok(out)
}

/// Insere um membro novo em um objeto, preservando o restante do texto.
pub fn insert_member(
    text: &str,
    parent: &[&str],
    key: &str,
    value: &str,
) -> Result<String, String> {
    let full = parent
        .iter()
        .copied()
        .chain(std::iter::once(key))
        .collect::<Vec<_>>();
    if find(text, &full)?.is_some() {
        return replace(text, &full, value);
    }

    let Some(object) = find(text, parent)? else {
        return Err(format!("objeto não encontrado: {}", parent.join(".")));
    };
    let inner = text[object.span.start..object.span.end].trim_start();
    let offset = object.span.start + (text[object.span.start..object.span.end].len() - inner.len());
    let trimmed = inner.trim_end();
    let empty = trimmed == "{}";
    let indent = detect_indent(text, &object.span);

    let mut out = String::with_capacity(text.len() + value.len() + 16);
    out.push_str(&text[..offset]);
    if empty {
        out.push_str("{\n");
        out.push_str(&indent);
        out.push_str(&format!("\"{key}\": {value}\n"));
        out.push_str(indent.trim_end_matches([' ', '\t']));
        out.push('}');
    } else {
        let close = offset + trimmed.len() - 2;
        out.push_str(&text[offset..close]);
        out.push_str(",\n");
        out.push_str(&indent);
        out.push_str(&format!("\"{key}\": {value}"));
        out.push_str(&text[close..]);
    }
    Ok(out)
}

/// A indentação usada dentro do objeto, inferida do documento.
fn detect_indent(text: &str, span: &Range<usize>) -> String {
    let body = &text[span.start..span.end];
    for line in body.lines().skip(1) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        if !indent.is_empty() {
            return format!("{indent}  ");
        }
    }
    "  ".to_string()
}

struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    pos: usize,
    nodes: Vec<Node>,
}

impl<'a> Parser<'a> {
    fn skip_trivia(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.text[self.pos..].starts_with("//") {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            if self.text[self.pos..].starts_with("/*") {
                let end = self.text[self.pos..].find("*/").map(|i| self.pos + i + 2);
                match end {
                    Some(end) => self.pos = end,
                    None => {
                        self.pos = self.bytes.len();
                        return;
                    }
                }
                continue;
            }
            return;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn value(&mut self, path: &mut Vec<String>) -> Result<Json, String> {
        self.skip_trivia();
        let start = self.pos;
        let value = match self.peek() {
            Some(b'{') => self.object(path)?,
            Some(b'[') => self.array(path)?,
            Some(b'"') => Json::String(self.string()?),
            Some(b't') => {
                self.expect("true")?;
                Json::Bool(true)
            }
            Some(b'f') => {
                self.expect("false")?;
                Json::Bool(false)
            }
            Some(b'n') => {
                self.expect("null")?;
                Json::Null
            }
            Some(_) => self.number()?,
            None => return Err("fim do documento inesperado".to_string()),
        };
        self.nodes.push(Node {
            path: path.clone(),
            span: start..self.pos,
            value: value.clone(),
        });
        Ok(value)
    }

    fn expect(&mut self, word: &str) -> Result<(), String> {
        if self.text[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(())
        } else {
            Err(format!("esperado {word} na posição {}", self.pos))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self
            .peek()
            .map(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'))
            .unwrap_or(false)
        {
            self.pos += 1;
        }
        self.text[start..self.pos]
            .parse::<f64>()
            .map(Json::Number)
            .map_err(|e| format!("número inválido: {e}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            self.pos += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self.peek().ok_or("escape incompleto")?;
                    self.pos += 1;
                    match escape {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = self
                                .text
                                .get(self.pos..self.pos + 4)
                                .ok_or("escape unicode")?;
                            let code = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
                            self.pos += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        other => out.push(other as char),
                    }
                }
                _ => {
                    let ch = self.text[self.pos - 1..]
                        .chars()
                        .next()
                        .unwrap_or('\u{fffd}');
                    out.push(ch);
                }
            }
        }
        Err("string sem fim".to_string())
    }

    fn array(&mut self, path: &mut Vec<String>) -> Result<Json, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek() == Some(b']') {
                self.pos += 1;
                break;
            }
            let item = self.value(path)?;
            items.push(item);
            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("vírgula esperada na posição {}", self.pos)),
            }
        }
        Ok(Json::Array(items))
    }

    fn object(&mut self, path: &mut Vec<String>) -> Result<Json, String> {
        self.pos += 1;
        let mut map = BTreeMap::new();
        loop {
            self.skip_trivia();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                break;
            }
            let key = self.string()?;
            self.skip_trivia();
            if self.peek() != Some(b':') {
                return Err(format!("dois pontos esperado na posição {}", self.pos));
            }
            self.pos += 1;
            path.push(key.clone());
            let value = self.value(path)?;
            path.pop();
            map.insert(key, value);
            self.skip_trivia();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("vírgula esperada na posição {}", self.pos)),
            }
        }
        Ok(Json::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_parsed_and_located() {
        let text = "{\n  \"mcp\": {\n    \"servers\": {\n      \"a\": {\"type\": \"local\"}\n    }\n  }\n}";
        let value = value(text).unwrap();
        assert!(value.get("mcp").is_some());
        let node = find(text, &["mcp", "servers", "a", "type"]).unwrap();
        assert!(node.is_some());
        assert_eq!(&text[node.unwrap().span.clone()], "\"local\"");
    }

    #[test]
    fn comments_are_ignored_by_the_parser() {
        let text = "{\n  // nota do usuário\n  \"a\": 1 /* inline */\n}";
        let value = value(text).unwrap();
        assert_eq!(value.get("a"), Some(&Json::Number(1.0)));
    }

    #[test]
    fn a_member_is_replaced_without_touching_the_rest() {
        let text = "{\n  // manter\n  \"a\": 1,\n  \"b\": 2\n}";
        let out = replace(text, &["a"], "9").unwrap();
        assert!(out.contains("\"a\": 9"));
        assert!(out.contains("// manter"));
        assert!(out.contains("\"b\": 2"));
    }

    #[test]
    fn a_member_is_inserted_preserving_the_existing_text() {
        let text = "{\n  // manter\n  \"a\": 1\n}";
        let out = insert_member(text, &[], "b", "2").unwrap();
        assert!(out.contains("// manter"));
        assert!(out.contains("\"b\": 2"));
        assert!(out.contains("\"a\": 1"));
    }

    #[test]
    fn a_member_is_inserted_in_an_empty_object() {
        let text = "{}";
        let out = insert_member(text, &[], "b", "2").unwrap();
        let parsed = value(&out).unwrap();
        assert_eq!(parsed.get("b"), Some(&Json::Number(2.0)));
    }

    #[test]
    fn an_existing_member_is_replaced_instead_of_duplicated() {
        let text = "{\n  \"a\": 1\n}";
        let out = insert_member(text, &[], "a", "2").unwrap();
        assert_eq!(out.matches("\"a\"").count(), 1);
        assert!(out.contains("\"a\": 2"));
    }
}
