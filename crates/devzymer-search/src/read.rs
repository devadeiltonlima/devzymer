use std::path::Path;

use serde::Serialize;

use devzymer_core::util::relative_slash;
use devzymer_core::Result;

#[derive(Debug, Clone)]
pub struct ExcerptRequest {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub max_bytes: usize,
    pub max_lines: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Excerpt {
    pub path: String,
    /// The first line delivered.
    pub start_line: u32,
    /// The last line delivered. It is never larger than the file, and it
    /// never promises a line that was not returned.
    pub end_line: u32,
    pub text: String,
    pub bytes: usize,
    pub lines: usize,
    pub truncated: bool,
    /// The line that continues the excerpt, when the range was cut.
    pub next_start_line: Option<u32>,
    pub sha: String,
}

/// Reads a line range from a file, clipped to the requested byte budget.
///
/// The file is streamed, so a huge file is never loaded whole. The excerpt
/// reports the last line it actually delivered, which is what a caller must
/// use to address the same block later.
pub fn read_excerpt(root: &Path, request: &ExcerptRequest) -> Result<Excerpt> {
    let abs = devzymer_core::util::resolve_inside(root, Path::new(&request.path))?;
    let file = std::fs::File::open(&abs).map_err(|e| devzymer_core::Error::io(&abs, e))?;
    let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);

    let mut selected: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    let start = request.start_line.max(1);
    let requested_end = request.end_line.max(start);
    // The last line actually delivered. It never promises a line that was
    // not returned, so a caller can address exactly what it received.
    let mut delivered_end = start;
    let mut current = 0u32;
    let mut buffer = String::new();

    loop {
        buffer.clear();
        let read = std::io::BufRead::read_line(&mut reader, &mut buffer)
            .map_err(|e| devzymer_core::Error::io(&abs, e))?;
        if read == 0 {
            break;
        }
        current += 1;
        if current < start {
            continue;
        }
        if current > requested_end {
            break;
        }
        if selected.len() >= request.max_lines.max(1) {
            truncated = true;
            break;
        }
        let line = buffer.trim_end_matches(['\n', '\r']);
        if bytes + line.len() + 1 > request.max_bytes {
            truncated = true;
            break;
        }
        bytes += line.len() + 1;
        selected.push(line.to_string());
        delivered_end = current;
    }

    let end = if selected.is_empty() {
        // Nothing was delivered: the range is past the end of the file, so the
        // reported end is the last line the file actually has.
        current.min(start)
    } else {
        delivered_end
    };
    let body = selected.join("\n");
    let sha = file_digest(&abs)?;
    let next_start_line = if truncated { Some(end + 1) } else { None };
    Ok(Excerpt {
        path: relative_slash(root, &abs).unwrap_or_else(|| request.path.clone()),
        start_line: start,
        end_line: end,
        sha,
        bytes: body.len(),
        lines: selected.len(),
        truncated,
        next_start_line,
        text: body,
    })
}

/// The digest of a file, read in bounded blocks.
fn file_digest(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| devzymer_core::Error::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| devzymer_core::Error::io(path, e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex()[..16].to_string())
}

/// Reads an entire file, used as the baseline in benchmark comparisons.
pub fn read_whole(root: &Path, path: &str) -> Result<(String, usize)> {
    let abs = devzymer_core::util::resolve_inside(root, Path::new(path))?;
    let content = std::fs::read(&abs).map_err(|e| devzymer_core::Error::io(&abs, e))?;
    let text = String::from_utf8_lossy(&content).into_owned();
    let size = text.len();
    Ok((text, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own directory, so parallel tests never write the
    /// same file at the same time.
    fn workspace(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("devzymer-read-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sample.rs"),
            "linha 1\nlinha 2\nlinha 3\nlinha 4\nlinha 5\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn excerpts_are_clipped_to_the_requested_range() {
        let root = workspace("range");
        let excerpt = read_excerpt(
            &root,
            &ExcerptRequest {
                path: "sample.rs".into(),
                start_line: 2,
                end_line: 4,
                max_bytes: 1000,
                max_lines: 100,
            },
        )
        .unwrap();
        assert_eq!(excerpt.text, "linha 2\nlinha 3\nlinha 4");
        assert_eq!(excerpt.start_line, 2);
        assert_eq!(excerpt.end_line, 4);
        assert!(!excerpt.truncated);
        assert!(!excerpt.sha.is_empty());
    }

    #[test]
    fn a_cut_excerpt_reports_the_last_delivered_line() {
        let root = workspace("cut");
        std::fs::write(
            root.join("long.txt"),
            (1..=100)
                .map(|i| format!("linha {i}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let excerpt = read_excerpt(
            &root,
            &ExcerptRequest {
                path: "long.txt".into(),
                start_line: 1,
                end_line: 100,
                max_bytes: 1_000_000,
                max_lines: 40,
            },
        )
        .unwrap();
        assert!(excerpt.truncated);
        assert_eq!(
            excerpt.end_line, 40,
            "o fim deve ser a ultima linha entregue"
        );
        assert_eq!(excerpt.lines, 40);
        assert_eq!(excerpt.next_start_line, Some(41));
    }

    #[test]
    fn a_range_past_the_end_is_reported_honestly() {
        let root = workspace("past-end");
        let excerpt = read_excerpt(
            &root,
            &ExcerptRequest {
                path: "sample.rs".into(),
                start_line: 90,
                end_line: 120,
                max_bytes: 1000,
                max_lines: 100,
            },
        )
        .unwrap();
        assert_eq!(excerpt.lines, 0);
        assert_eq!(excerpt.end_line, 5, "o fim real do arquivo");
        assert!(!excerpt.truncated);
    }

    #[test]
    fn excerpts_respect_the_byte_budget() {
        let root = workspace("budget");
        let excerpt = read_excerpt(
            &root,
            &ExcerptRequest {
                path: "sample.rs".into(),
                start_line: 1,
                end_line: 5,
                max_bytes: 12,
                max_lines: 100,
            },
        )
        .unwrap();
        assert!(excerpt.truncated);
        assert!(excerpt.bytes <= 12);
    }

    #[test]
    fn paths_outside_the_workspace_are_rejected() {
        let root = workspace("escape");
        let outcome = read_excerpt(
            &root,
            &ExcerptRequest {
                path: "../fora.txt".into(),
                start_line: 1,
                end_line: 1,
                max_bytes: 100,
                max_lines: 10,
            },
        );
        assert!(outcome.is_err());
    }
}
