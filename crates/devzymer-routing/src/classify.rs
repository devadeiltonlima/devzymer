use std::path::Path;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    CodeSearch,
    FileDiscovery,
    FileRead,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: IntentKind,
    pub category: &'static str,
    pub terms: Vec<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone)]
struct Word {
    text: String,
    quoted: bool,
}

#[derive(Debug)]
struct Segment {
    text: String,
    after_pipe: bool,
}

const MAX_DEPTH: u8 = 2;
const MAX_TERMS: usize = 4;

pub fn classify(tool: &str, input: &Value, cwd: &Path) -> Option<Intent> {
    match tool.to_ascii_lowercase().as_str() {
        "grep" => {
            let pattern = text_field(input, &["pattern"])?;
            Some(Intent {
                kind: IntentKind::CodeSearch,
                category: "grep-tool",
                terms: identifier_terms(&pattern),
                scope: text_field(input, &["path"]).filter(|path| !is_current_dir(path)),
            })
        }
        "glob" => {
            let pattern = text_field(input, &["pattern"])?;
            if has_wildcard(&pattern) {
                return None;
            }
            Some(Intent {
                kind: IntentKind::FileDiscovery,
                category: "glob-tool",
                terms: file_terms(&pattern),
                scope: text_field(input, &["path"]).filter(|path| !is_current_dir(path)),
            })
        }
        "read" => {
            let path = text_field(input, &["file_path", "filePath", "path"])?;
            Some(Intent {
                kind: IntentKind::FileRead,
                category: "read-tool",
                terms: Vec::new(),
                scope: Some(path),
            })
        }
        "bash" | "powershell" | "shell" | "exec_command" | "local_shell" => {
            let command = text_field(input, &["command", "cmd"])?;
            classify_shell(&command, cwd)
        }
        _ => None,
    }
}

pub fn classify_shell(command: &str, cwd: &Path) -> Option<Intent> {
    classify_depth(command, cwd, 0)
}

fn text_field(input: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn is_current_dir(path: &str) -> bool {
    matches!(path.trim(), "." | "./" | ".\\" | "")
}

fn has_wildcard(text: &str) -> bool {
    text.contains(['*', '?', '[', '{'])
}

fn classify_depth(command: &str, cwd: &Path, depth: u8) -> Option<Intent> {
    let segments = split_segments(command);
    let mut previous: Vec<Word> = Vec::new();
    let mut best: Option<Intent> = None;
    for segment in &segments {
        let (words, stdin_redirect) = tokenize(&segment.text);
        if let Some(intent) = classify_words(
            &words,
            segment.after_pipe,
            stdin_redirect,
            &previous,
            cwd,
            depth,
        ) {
            let better = match &best {
                None => true,
                Some(current) => rank(intent.kind) > rank(current.kind),
            };
            if better {
                best = Some(intent);
            }
        }
        previous = words;
    }
    best
}

fn split_segments(command: &str) -> Vec<Segment> {
    let chars: Vec<char> = command.chars().collect();
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_after_pipe = false;
    let mut single = false;
    let mut double = false;
    let mut depth = 0i32;
    let mut index = 0;

    let flush = |current: &mut String, after_pipe: bool, segments: &mut Vec<Segment>| {
        if !current.trim().is_empty() {
            segments.push(Segment {
                text: std::mem::take(current),
                after_pipe,
            });
        } else {
            current.clear();
        }
    };

    while index < chars.len() {
        let c = chars[index];
        let next = chars.get(index + 1).copied();
        if single {
            current.push(c);
            if c == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            current.push(c);
            if c == '\\' {
                if let Some(escaped) = next {
                    current.push(escaped);
                    index += 2;
                    continue;
                }
            }
            if c == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        match c {
            '\'' => {
                single = true;
                current.push(c);
            }
            '"' => {
                double = true;
                current.push(c);
            }
            '`' => {
                current.push(c);
                if let Some(escaped) = next {
                    current.push(escaped);
                    index += 1;
                }
            }
            '$' if next == Some('(') => {
                depth += 1;
                current.push('$');
                current.push('(');
                index += 1;
            }
            '(' if depth > 0 => {
                depth += 1;
                current.push(c);
            }
            ')' if depth > 0 => {
                depth -= 1;
                current.push(c);
            }
            '|' if depth == 0 => {
                flush(&mut current, current_after_pipe, &mut segments);
                if next == Some('|') {
                    current_after_pipe = false;
                    index += 1;
                } else {
                    current_after_pipe = true;
                }
            }
            '&' if depth == 0 && next == Some('&') => {
                flush(&mut current, current_after_pipe, &mut segments);
                current_after_pipe = false;
                index += 1;
            }
            ';' | '\n' | '\r' if depth == 0 => {
                flush(&mut current, current_after_pipe, &mut segments);
                current_after_pipe = false;
            }
            _ => current.push(c),
        }
        index += 1;
    }
    flush(&mut current, current_after_pipe, &mut segments);
    segments
}

fn tokenize(text: &str) -> (Vec<Word>, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut raw: Vec<Word> = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut single = false;
    let mut double = false;
    let mut index = 0;

    while index < chars.len() {
        let c = chars[index];
        if single {
            if c == '\'' {
                single = false;
            } else {
                current.push(c);
            }
            index += 1;
            continue;
        }
        if double {
            if c == '\\' && matches!(chars.get(index + 1), Some('"') | Some('\\')) {
                current.push(chars[index + 1]);
                index += 2;
                continue;
            }
            if c == '"' {
                double = false;
            } else {
                current.push(c);
            }
            index += 1;
            continue;
        }
        match c {
            '\'' => {
                single = true;
                in_word = true;
                quoted = true;
            }
            '"' => {
                double = true;
                in_word = true;
                quoted = true;
            }
            '`' => {
                if let Some(escaped) = chars.get(index + 1) {
                    current.push(*escaped);
                    in_word = true;
                    index += 1;
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    raw.push(Word {
                        text: std::mem::take(&mut current),
                        quoted,
                    });
                    in_word = false;
                    quoted = false;
                }
            }
            _ => {
                current.push(c);
                in_word = true;
            }
        }
        index += 1;
    }
    if in_word {
        raw.push(Word {
            text: current,
            quoted,
        });
    }

    let mut words = Vec::new();
    let mut stdin_redirect = false;
    let mut skip_next = false;
    for word in raw {
        if skip_next {
            skip_next = false;
            continue;
        }
        if !word.quoted {
            let text = word.text.as_str();
            if text.starts_with('<') {
                stdin_redirect = true;
                skip_next = text == "<";
                continue;
            }
            if let Some(consumes_next) = output_redirect(text) {
                skip_next = consumes_next;
                continue;
            }
        }
        words.push(word);
    }
    (words, stdin_redirect)
}

fn output_redirect(text: &str) -> Option<bool> {
    let rest = text.trim_start_matches(|c: char| c.is_ascii_digit());
    let after = rest.strip_prefix("&>").or_else(|| rest.strip_prefix('>'))?;
    let after = after.strip_prefix('>').unwrap_or(after);
    Some(after.is_empty())
}

fn base_command(word: &str) -> String {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = name.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stripped) = lower.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    lower
}

fn is_assignment(word: &Word) -> bool {
    if word.quoted {
        return false;
    }
    match word.text.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn classify_words(
    words: &[Word],
    after_pipe: bool,
    stdin_redirect: bool,
    previous: &[Word],
    cwd: &Path,
    depth: u8,
) -> Option<Intent> {
    let mut index = 0;
    while index < words.len() {
        let word = &words[index];
        let lower = word.text.to_ascii_lowercase();
        if is_assignment(word)
            || matches!(
                lower.as_str(),
                "sudo"
                    | "time"
                    | "nohup"
                    | "command"
                    | "builtin"
                    | "exec"
                    | "env"
                    | "nice"
                    | "&"
                    | "call"
            )
        {
            index += 1;
            continue;
        }
        break;
    }
    let first = words.get(index)?;
    let command = base_command(&first.text);
    let args = &words[index + 1..];

    match command.as_str() {
        "bash" | "sh" | "zsh" | "dash" | "ksh" => {
            let inner = args
                .iter()
                .position(|arg| arg.text.starts_with('-') && arg.text.contains('c'))
                .and_then(|position| args.get(position + 1))?;
            nested(&inner.text, cwd, depth)
        }
        "cmd" => {
            let position = args
                .iter()
                .position(|arg| matches!(arg.text.to_ascii_lowercase().as_str(), "/c" | "/k"))?;
            let inner = join_words(&args[position + 1..]);
            nested(&inner, cwd, depth)
        }
        "powershell" | "pwsh" => {
            let position = args.iter().position(|arg| {
                matches!(
                    arg.text.to_ascii_lowercase().as_str(),
                    "-c" | "-command" | "-com"
                )
            })?;
            let inner = join_words(&args[position + 1..]);
            nested(&inner, cwd, depth)
        }
        "xargs" => {
            let inner_start = args.iter().position(|arg| !arg.text.starts_with('-'))?;
            let inner = &args[inner_start..];
            let name = base_command(&inner.first()?.text);
            if matches!(
                name.as_str(),
                "grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack"
            ) {
                let mut intent = grep_like(&name, &inner[1..], true, false, false, cwd)?;
                if intent.kind == IntentKind::CodeSearch {
                    intent.scope = None;
                }
                Some(intent)
            } else {
                None
            }
        }
        "grep" | "egrep" | "fgrep" | "zgrep" | "ugrep" | "ug" => {
            grep_like(&command, args, false, after_pipe, stdin_redirect, cwd)
        }
        "rg" | "ripgrep" | "ag" | "ack" | "pt" => {
            grep_like(&command, args, true, after_pipe, stdin_redirect, cwd)
        }
        "git" => {
            let mut position = 0;
            while position < args.len() {
                let text = args[position].text.as_str();
                if matches!(
                    text,
                    "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
                ) {
                    position += 2;
                } else if text.starts_with('-') {
                    position += 1;
                } else {
                    break;
                }
            }
            if args.get(position)?.text == "grep" {
                git_grep(&args[position + 1..])
            } else {
                None
            }
        }
        "findstr" => findstr(args, after_pipe, cwd),
        "select-string" | "sls" => select_string(args, after_pipe, previous, cwd),
        "find" => find_command(args),
        "fd" | "fdfind" => fd_command(args),
        "get-childitem" | "gci" => child_item(args),
        _ => None,
    }
}

fn rank(kind: IntentKind) -> u8 {
    match kind {
        IntentKind::CodeSearch => 3,
        IntentKind::FileDiscovery => 2,
        IntentKind::FileRead => 1,
    }
}

fn nested(command: &str, cwd: &Path, depth: u8) -> Option<Intent> {
    if depth >= MAX_DEPTH {
        return None;
    }
    classify_depth(command, cwd, depth + 1)
}

fn join_words(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_repo_path(path: &str, cwd: &Path) -> bool {
    let trimmed = path.trim();
    if matches!(trimmed, "." | ".." | "./" | ".\\") {
        return true;
    }
    if trimmed.ends_with('/') || trimmed.ends_with('\\') || has_wildcard(trimmed) {
        return true;
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        candidate.is_dir()
    } else {
        cwd.join(candidate).is_dir()
    }
}

fn grep_like(
    command: &str,
    args: &[Word],
    recursive_default: bool,
    after_pipe: bool,
    stdin_redirect: bool,
    cwd: &Path,
) -> Option<Intent> {
    let (short_values, recursive_chars): (&str, &str) = if recursive_default {
        ("efgtTmABCjMEr", "")
    } else {
        ("efmABCdD", "rR")
    };
    let long_values = [
        "--include",
        "--exclude",
        "--exclude-dir",
        "--max-count",
        "--context",
        "--after-context",
        "--before-context",
        "--label",
        "--directories",
        "--devices",
        "--glob",
        "--iglob",
        "--type",
        "--type-not",
        "--type-add",
        "--threads",
        "--max-depth",
        "--replace",
        "--encoding",
        "--sort",
        "--sortr",
        "--pre",
        "--max-filesize",
        "--max-columns",
    ];

    let mut pattern: Option<String> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut recursive = recursive_default;
    let mut files_mode = false;
    let mut only_positional = false;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        let text = arg.text.as_str();
        if !only_positional && !arg.quoted && text == "--" {
            only_positional = true;
            index += 1;
            continue;
        }
        if !only_positional && !arg.quoted && text.starts_with("--") {
            let (name, value) = match text.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (text, None),
            };
            match name {
                "--recursive" | "--dereference-recursive" => recursive = true,
                "--files" => files_mode = true,
                "--regexp" | "--file" => {
                    pattern = match value {
                        Some(value) => Some(value),
                        None => {
                            index += 1;
                            Some(args.get(index).map(|a| a.text.clone()).unwrap_or_default())
                        }
                    };
                }
                _ if value.is_none() && long_values.contains(&name) => index += 1,
                _ => {}
            }
            index += 1;
            continue;
        }
        if !only_positional && !arg.quoted && text.starts_with('-') && text.len() > 1 {
            let chars: Vec<char> = text[1..].chars().collect();
            let mut position = 0;
            let mut consumed_next = false;
            while position < chars.len() {
                let c = chars[position];
                if recursive_chars.contains(c) {
                    recursive = true;
                }
                if c == 'e' || c == 'f' {
                    let attached: String = chars[position + 1..].iter().collect();
                    pattern = if attached.is_empty() {
                        consumed_next = true;
                        Some(
                            args.get(index + 1)
                                .map(|a| a.text.clone())
                                .unwrap_or_default(),
                        )
                    } else {
                        Some(attached)
                    };
                    break;
                }
                if short_values.contains(c) {
                    if position + 1 == chars.len() {
                        consumed_next = true;
                    }
                    break;
                }
                position += 1;
            }
            index += 1 + usize::from(consumed_next);
            continue;
        }
        if pattern.is_none() {
            pattern = Some(arg.text.clone());
        } else {
            paths.push(arg.text.clone());
        }
        index += 1;
    }

    if files_mode {
        return Some(Intent {
            kind: IntentKind::FileDiscovery,
            category: category_of(command),
            terms: Vec::new(),
            scope: None,
        });
    }
    let pattern = pattern?;

    let repo_scope = if paths.is_empty() {
        if recursive_default {
            !after_pipe && !stdin_redirect
        } else {
            recursive && !after_pipe && !stdin_redirect
        }
    } else {
        paths.iter().any(|path| is_repo_path(path, cwd))
    };
    if !repo_scope {
        return None;
    }
    let scope = if paths.len() == 1 && !is_current_dir(&paths[0]) {
        Some(paths[0].clone())
    } else {
        None
    };
    Some(Intent {
        kind: IntentKind::CodeSearch,
        category: category_of(command),
        terms: identifier_terms(&pattern),
        scope,
    })
}

fn category_of(command: &str) -> &'static str {
    match command {
        "rg" | "ripgrep" => "rg",
        "ag" => "ag",
        "ack" => "ack",
        "pt" => "pt",
        "ugrep" | "ug" => "ugrep",
        _ => "grep",
    }
}

fn git_grep(args: &[Word]) -> Option<Intent> {
    let pattern = args
        .iter()
        .find(|arg| arg.quoted || !arg.text.starts_with('-'))
        .map(|arg| arg.text.clone())?;
    Some(Intent {
        kind: IntentKind::CodeSearch,
        category: "git-grep",
        terms: identifier_terms(&pattern),
        scope: None,
    })
}

fn findstr(args: &[Word], after_pipe: bool, cwd: &Path) -> Option<Intent> {
    let mut pattern: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut recursive = false;
    for arg in args {
        let lower = arg.text.to_ascii_lowercase();
        if !arg.quoted && arg.text.starts_with('/') && arg.text.len() > 1 {
            if lower == "/s" {
                recursive = true;
            } else if lower.starts_with("/c:") {
                pattern = Some(arg.text[3..].to_string());
            }
            continue;
        }
        if pattern.is_none() {
            pattern = Some(arg.text.clone());
        } else {
            files.push(arg.text.clone());
        }
    }
    let pattern = pattern?;
    let repo_scope = if files.is_empty() {
        recursive && !after_pipe
    } else {
        recursive || files.iter().any(|file| is_repo_path(file, cwd))
    };
    if !repo_scope {
        return None;
    }
    Some(Intent {
        kind: IntentKind::CodeSearch,
        category: "findstr",
        terms: identifier_terms(&pattern),
        scope: None,
    })
}

fn is_recursive_listing(words: &[Word]) -> bool {
    let Some(first) = words.iter().find(|word| !is_assignment(word)) else {
        return false;
    };
    let name = base_command(&first.text);
    if !matches!(name.as_str(), "get-childitem" | "gci" | "ls" | "dir") {
        return false;
    }
    words.iter().any(|word| {
        let lower = word.text.to_ascii_lowercase();
        lower == "-recurse" || lower == "-r" || lower == "-rec"
    })
}

fn select_string(args: &[Word], after_pipe: bool, previous: &[Word], cwd: &Path) -> Option<Intent> {
    let value_params = [
        "-include",
        "-exclude",
        "-encoding",
        "-context",
        "-inputobject",
    ];
    let mut pattern: Option<String> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut positional = 0;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let lower = arg.text.to_ascii_lowercase();
        if !arg.quoted && arg.text.starts_with('-') && arg.text.len() > 1 {
            if lower == "-pattern" || lower == "-pat" {
                index += 1;
                pattern = args.get(index).map(|a| a.text.clone());
            } else if lower == "-path" || lower == "-literalpath" {
                index += 1;
                if let Some(value) = args.get(index) {
                    paths.extend(value.text.split(',').map(|p| p.trim().to_string()));
                }
            } else if value_params.contains(&lower.as_str()) {
                index += 1;
            }
            index += 1;
            continue;
        }
        match positional {
            0 if pattern.is_none() => pattern = Some(arg.text.clone()),
            _ => paths.extend(arg.text.split(',').map(|p| p.trim().to_string())),
        }
        positional += 1;
        index += 1;
    }
    let pattern = pattern?;
    let repo_scope = if paths.is_empty() {
        after_pipe && is_recursive_listing(previous)
    } else {
        paths.iter().any(|path| is_repo_path(path, cwd))
    };
    if !repo_scope {
        return None;
    }
    Some(Intent {
        kind: IntentKind::CodeSearch,
        category: "select-string",
        terms: identifier_terms(&pattern),
        scope: None,
    })
}

fn find_command(args: &[Word]) -> Option<Intent> {
    let mut name: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let lower = args[index].text.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete" | "-fprint" | "-fls"
        ) {
            return None;
        }
        if matches!(
            lower.as_str(),
            "-name" | "-iname" | "-path" | "-ipath" | "-wholename" | "-regex" | "-iregex"
        ) && name.is_none()
        {
            name = args.get(index + 1).map(|arg| arg.text.clone());
        }
        index += 1;
    }
    let name = name?;
    Some(Intent {
        kind: IntentKind::FileDiscovery,
        category: "find",
        terms: file_terms(&name),
        scope: None,
    })
}

fn fd_command(args: &[Word]) -> Option<Intent> {
    let pattern = args
        .iter()
        .find(|arg| arg.quoted || !arg.text.starts_with('-'))
        .map(|arg| arg.text.clone())?;
    Some(Intent {
        kind: IntentKind::FileDiscovery,
        category: "fd",
        terms: file_terms(&pattern),
        scope: None,
    })
}

fn child_item(args: &[Word]) -> Option<Intent> {
    let recursive = args.iter().any(|arg| {
        let lower = arg.text.to_ascii_lowercase();
        lower == "-recurse" || lower == "-r" || lower == "-rec"
    });
    if !recursive {
        return None;
    }
    let mut value: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let lower = args[index].text.to_ascii_lowercase();
        if matches!(lower.as_str(), "-filter" | "-include" | "-fi" | "-in") {
            value = args.get(index + 1).map(|arg| arg.text.clone());
            break;
        }
        index += 1;
    }
    let value = value?;
    Some(Intent {
        kind: IntentKind::FileDiscovery,
        category: "get-childitem",
        terms: file_terms(&value),
        scope: None,
    })
}

pub fn identifier_terms(pattern: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in pattern.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '$') {
        let candidate = raw.trim_matches(|c| c == '.' || c == '$');
        if candidate.chars().count() < 4 {
            continue;
        }
        if !looks_like_identifier(candidate) {
            continue;
        }
        if !terms.iter().any(|existing| existing == candidate) {
            terms.push(candidate.to_string());
        }
        if terms.len() >= MAX_TERMS {
            break;
        }
    }
    terms
}

fn looks_like_identifier(candidate: &str) -> bool {
    if candidate.contains('_') || candidate.contains('.') {
        return true;
    }
    let has_lower = candidate.chars().any(|c| c.is_lowercase());
    let inner_upper = candidate.chars().skip(1).any(|c| c.is_uppercase());
    has_lower && inner_upper
}

pub fn file_terms(pattern: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let name = pattern.rsplit(['/', '\\']).next().unwrap_or(pattern);
    for raw in name.split(['*', '?', '[', ']', '{', '}', ',']) {
        let candidate = raw.trim_matches('.');
        let stem = match candidate.rfind('.') {
            Some(position) if position > 0 => &candidate[..position],
            _ => candidate,
        };
        if stem.chars().count() < 4 {
            continue;
        }
        if !terms.iter().any(|existing| existing == stem) {
            terms.push(stem.to_string());
        }
        if terms.len() >= MAX_TERMS {
            break;
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dz-classify-{}", devzymer_core::util::now_millis()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();
        dir
    }

    fn shell(command: &str, cwd: &Path) -> Option<(IntentKind, &'static str)> {
        classify("Bash", &json!({ "command": command }), cwd)
            .map(|intent| (intent.kind, intent.category))
    }

    #[test]
    fn a_recursive_text_search_of_the_repository_is_recognized() {
        let cwd = fixture();
        for (command, category) in [
            ("grep -rn \"useVoiceCall\" src/", "grep"),
            ("grep -r foo .", "grep"),
            ("grep -rn foo . | head -20", "grep"),
            ("rg useVoiceCall", "rg"),
            ("rg -n \"foo\" .", "rg"),
            ("rg Foo src", "rg"),
            ("cd src && rg Foo", "rg"),
            ("FOO=1 rg Foo", "rg"),
            ("sudo grep -R Foo .", "grep"),
            ("git grep -n useVoiceCall", "git-grep"),
            ("git -C repo grep Foo", "git-grep"),
            ("findstr /s /i \"useVoiceCall\" *.ts", "findstr"),
            (
                "Select-String -Path \"src\\*.ts\" -Pattern \"useVoiceCall\"",
                "select-string",
            ),
            (
                "Get-ChildItem -Recurse -Include *.ts | Select-String useVoiceCall",
                "select-string",
            ),
            ("gci -r | sls Foo", "select-string"),
            ("git ls-files | xargs grep -n FooBar", "grep"),
            ("bash -c \"rg useVoiceCall\"", "rg"),
            ("sh -lc 'grep -rn Foo .'", "grep"),
            (
                "powershell -Command \"Select-String -Path src\\*.ts -Pattern foo\"",
                "select-string",
            ),
            ("cmd /c findstr /s foo *.ts", "findstr"),
        ] {
            assert_eq!(
                shell(command, &cwd),
                Some((IntentKind::CodeSearch, category)),
                "{command}"
            );
        }
    }

    #[test]
    fn a_file_name_lookup_is_recognized_as_file_discovery() {
        let cwd = fixture();
        for (command, category) in [
            ("find . -name \"useVoiceCall.ts\"", "find"),
            ("find src -iname 'UserService*' -type f", "find"),
            ("fd useVoiceCall", "fd"),
            (
                "Get-ChildItem -Recurse -Filter useVoiceCall.ts",
                "get-childitem",
            ),
            ("rg --files", "rg"),
            ("rg --files | rg Foo", "rg"),
        ] {
            assert_eq!(
                shell(command, &cwd),
                Some((IntentKind::FileDiscovery, category)),
                "{command}"
            );
        }
    }

    #[test]
    fn output_filters_and_single_files_are_not_repository_searches() {
        let cwd = fixture();
        for command in [
            "npm test 2>&1 | grep FAIL",
            "docker logs app | grep error",
            "cargo build 2>&1 | rg warning",
            "cat file | grep -n foo",
            "ls -la | grep src",
            "git log --oneline | grep fix",
            "grep -c foo < input.txt",
            "grep error build.log",
            "rg foo main.rs",
            "Get-Content app.log | Select-String error",
            "Select-String -Path app.log -Pattern error",
            "findstr foo notes.txt",
            "findstr /i foo",
        ] {
            assert_eq!(shell(command, &cwd), None, "{command}");
        }
    }

    #[test]
    fn commands_that_merely_mention_a_search_tool_are_ignored() {
        let cwd = fixture();
        for command in [
            "echo \"grep -r foo .\"",
            "git commit -m \"fix grep usage\"",
            "man grep",
            "npm run grep",
            "python tool.py --pattern grep",
            "cargo test",
            "git status",
            "ls -la",
            "grep",
            "rg",
            "",
        ] {
            assert_eq!(shell(command, &cwd), None, "{command}");
        }
    }

    #[test]
    fn a_destructive_find_is_never_treated_as_a_lookup() {
        let cwd = fixture();
        for command in [
            "find . -name \"*.log\" -delete",
            "find . -name x -exec rm {} \\;",
            "find . -type f",
        ] {
            assert_eq!(shell(command, &cwd), None, "{command}");
        }
    }

    #[test]
    fn the_search_terms_come_from_the_pattern_without_keeping_it() {
        let cwd = fixture();
        let intent = classify(
            "Grep",
            &json!({ "pattern": "useVoiceCall|TOKEN_DO_X", "path": "src" }),
            &cwd,
        )
        .unwrap();
        assert_eq!(intent.kind, IntentKind::CodeSearch);
        assert_eq!(intent.terms, vec!["useVoiceCall", "TOKEN_DO_X"]);
        assert_eq!(intent.scope.as_deref(), Some("src"));
        for plain in ["TODO", "error", "DEBUG", "Foo", "a.b"] {
            let intent = classify("Grep", &json!({ "pattern": plain }), &cwd).unwrap();
            assert!(
                intent.terms.is_empty() || plain == "a.b",
                "{plain}: {:?}",
                intent.terms
            );
        }
        let snake = classify("Grep", &json!({ "pattern": "MAX_RETRY_COUNT" }), &cwd).unwrap();
        assert_eq!(snake.terms, vec!["MAX_RETRY_COUNT"]);
        let pascal = classify("Grep", &json!({ "pattern": "UserService" }), &cwd).unwrap();
        assert_eq!(pascal.terms, vec!["UserService"]);
    }

    #[test]
    fn a_glob_without_wildcards_is_a_name_lookup() {
        let cwd = fixture();
        let intent = classify("Glob", &json!({ "pattern": "useVoiceCall.ts" }), &cwd).unwrap();
        assert_eq!(intent.kind, IntentKind::FileDiscovery);
        assert_eq!(intent.terms, vec!["useVoiceCall"]);
        assert!(classify("Glob", &json!({ "pattern": "**/*.ts" }), &cwd).is_none());
    }

    #[test]
    fn the_read_tool_reports_the_requested_file() {
        let cwd = fixture();
        let intent = classify("Read", &json!({ "file_path": "src/lib.rs" }), &cwd).unwrap();
        assert_eq!(intent.kind, IntentKind::FileRead);
        assert_eq!(intent.scope.as_deref(), Some("src/lib.rs"));
        let camel = classify("read", &json!({ "filePath": "a.ts" }), &cwd).unwrap();
        assert_eq!(camel.scope.as_deref(), Some("a.ts"));
    }

    #[test]
    fn unknown_tools_and_malformed_input_are_ignored() {
        let cwd = fixture();
        assert!(classify("WebFetch", &json!({ "url": "x" }), &cwd).is_none());
        assert!(classify("Bash", &json!({}), &cwd).is_none());
        assert!(classify("Bash", &json!({ "command": 5 }), &cwd).is_none());
        assert!(classify("Grep", &json!(null), &cwd).is_none());
    }

    #[test]
    fn nesting_is_bounded() {
        let cwd = fixture();
        let nested = "bash -c \"bash -c \\\"bash -c 'rg Foo'\\\"\"";
        assert_eq!(shell(nested, &cwd), None);
    }

    #[test]
    fn quoted_separators_do_not_split_a_command() {
        let cwd = fixture();
        assert_eq!(
            shell("rg \"a|b;c&&d\" .", &cwd),
            Some((IntentKind::CodeSearch, "rg"))
        );
        assert_eq!(shell("echo \"x\" | grep \"a|b\"", &cwd), None);
    }
}
