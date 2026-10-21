import pathlib

# ---------------- config: escrita atomica com nome unico ----------------
cfg = pathlib.Path("crates/devzymer-core/src/config.rs")
text = cfg.read_text(encoding="utf-8")
old = """pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("devzymer")
    ));
    std::fs::write(&tmp, bytes).map_err(|e| Error::io(&tmp, e))?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(Error::io(path, e))
        }
    }
}"""
new = """pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    // The temporary name carries the process id, so two processes writing the
    // same file never share a temporary file.
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("devzymer"),
        std::process::id()
    ));
    {
        use std::io::Write;

        let mut file = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        file.write_all(bytes).map_err(|e| Error::io(&tmp, e))?;
        file.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(Error::io(path, e))
        }
    }
}"""
assert text.count(old) == 1, "write_atomic"
cfg.write_text(text.replace(old, new), encoding="utf-8", newline="\n")
print("write_atomic com nome unico")
