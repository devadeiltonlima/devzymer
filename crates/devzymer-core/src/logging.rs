use std::path::Path;

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

use crate::config::Config;

pub fn init(config: &Config) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(normalize_level(&config.logging.level)));
    let registry = tracing_subscriber::registry().with(filter);
    let installed = match config.logging.file.as_ref() {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                Ok(file) => registry
                    .with(fmt::layer().with_writer(file).with_ansi(false))
                    .with(fmt::layer().with_writer(std::io::stderr))
                    .try_init()
                    .is_ok(),
                Err(_) => registry
                    .with(fmt::layer().with_writer(std::io::stderr))
                    .try_init()
                    .is_ok(),
            }
        }
        None => registry
            .with(fmt::layer().with_writer(std::io::stderr))
            .try_init()
            .is_ok(),
    };
    let _ = installed;
}

pub fn init_default() {
    let config = Config::default();
    init(&config);
}

fn normalize_level(level: &str) -> String {
    let l = level.trim().to_ascii_lowercase();
    match l.as_str() {
        "trace" | "debug" | "info" | "warn" | "warning" | "error" => l,
        "off" | "none" => "off".to_string(),
        _ => "info".to_string(),
    }
}

/// Removes secret-looking values from a string before it reaches a log sink.
pub fn redact(text: &str) -> String {
    if !crate::util::looks_secret(text) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for token in text.split_whitespace() {
        if crate::util::looks_secret(token) {
            out.push_str("[REDACTED]");
        } else {
            out.push_str(token);
        }
        out.push(' ');
    }
    out.trim_end().to_string()
}

/// Best effort check that a path is readable, used by the doctor command.
pub fn is_readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}
