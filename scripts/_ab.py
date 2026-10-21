import pathlib
import shutil
import subprocess
import sys

STORE = pathlib.Path("crates/devzymer-core/src/store.rs")
BACKUP = pathlib.Path("scripts/_store_backup.rs")

shutil.copy(STORE, BACKUP)

text = STORE.read_text(encoding="utf-8")
old = """            if path_like {
                terms.push(format!("{part}*"));
                continue;
            }
            for word in part.split('_') {"""
new = """            if path_like || true {
                terms.push(format!("{part}*"));
                continue;
            }
            for word in part.split('_') {"""
assert text.count(old) == 1
STORE.write_text(text.replace(old, new), encoding="utf-8", newline="\n")
print("split desativado temporariamente")
