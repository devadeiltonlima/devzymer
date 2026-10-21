//! File and content rules that keep secrets out of the index.
//!
//! The rules are conservative: a file is only skipped when its name matches
//! a well known secret pattern, or when its content carries a high confidence
//! credential marker. A pattern never replaces the content check, and the
//! content check never replaces the operator control.

/// The file names that carry credentials or personal data by convention.
pub const SECRET_FILE_PATTERNS: &[&str] = &[
    ".env",
    ".env.*",
    "*.env",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    "*.kdbx",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".npmrc",
    ".pypirc",
    ".netrc",
    "_netrc",
    ".htpasswd",
    ".pgpass",
    "credentials",
    "credentials.*",
    "*credentials*.json",
    "*credentials*.txt",
    "secrets.json",
    "secrets.yaml",
    "secrets.yml",
    "secrets.toml",
    "secrets.txt",
    "service-account*.json",
    "master.key",
    "*.sqlite",
    "*.sqlite3",
    "*.db",
    "*-wal",
    "*-shm",
    "*.dump",
    "Cookies",
    "Login Data",
    "Local State",
    "Web Data",
];

/// The number of bytes inspected when the content of a file is checked.
pub const CONTENT_SCAN_BYTES: usize = 64 * 1024;

/// True when the file name matches a known secret pattern.
///
/// The comparison ignores the case and works on the file name alone, so a
/// directory called `keys` does not exclude the files inside it.
pub fn is_secret_file_name(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let file = normalized.rsplit('/').next().unwrap_or(&normalized);
    let lower = file.to_ascii_lowercase();
    for pattern in SECRET_FILE_PATTERNS {
        let pattern = pattern.to_ascii_lowercase();
        if pattern.contains('*') {
            if glob_match(&pattern, &lower) {
                return true;
            }
        } else if lower == pattern {
            return true;
        }
    }
    false
}

/// A glob that only needs `*` and `?`, matched against the whole name.
fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let (mut p, mut v) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = p;
            mark = v;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            v = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// True when the content carries a high confidence credential marker.
pub fn looks_secret_content(text: &str) -> bool {
    let bounded: String = text.chars().take(CONTENT_SCAN_BYTES).collect();
    let markers = [
        "-----BEGIN RSA PRIVATE KEY",
        "-----BEGIN PRIVATE KEY",
        "-----BEGIN EC PRIVATE KEY",
        "-----BEGIN OPENSSH PRIVATE KEY",
        "-----BEGIN PGP PRIVATE KEY",
        "AKIA",
        "ghp_",
        "gho_",
        "ghu_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AIza",
        "sk-",
    ];
    markers.iter().any(|marker| bounded.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_file_name_is_recognized() {
        assert!(is_secret_file_name(".env"));
        assert!(is_secret_file_name("config/.env.production"));
        assert!(is_secret_file_name("certs/server.key"));
        assert!(is_secret_file_name("credentials.json"));
        assert!(is_secret_file_name("state.sqlite3"));
        assert!(is_secret_file_name("index.sqlite3-wal"));
    }

    #[test]
    fn an_ordinary_file_is_kept() {
        assert!(!is_secret_file_name("src/index.ts"));
        assert!(!is_secret_file_name("src/tokens.ts"));
        assert!(!is_secret_file_name("README.md"));
        assert!(!is_secret_file_name("src/credentials-manager.ts"));
    }

    #[test]
    fn a_secret_content_is_recognized() {
        assert!(looks_secret_content("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(looks_secret_content(
            "key = \"ghp_abcdefghijklmnopqrstuvwxyz012345\""
        ));
        assert!(!looks_secret_content(
            "export function calculate() { return 1; }"
        ));
    }

    #[test]
    fn the_glob_matches_the_whole_name() {
        assert!(glob_match("*.env", "production.env"));
        assert!(!glob_match("*.env", "production.envx"));
        assert!(glob_match("*-wal", "index.sqlite3-wal"));
    }
}
