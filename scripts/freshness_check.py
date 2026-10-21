"""Confirms that the index matches the source files right now.

For each indexed symbol of the sampled files, the line range reported by the
index is compared with the identifier found in the file on disk.
"""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
_NAME = "devzymer.exe" if os.name == "nt" else "devzymer"


def _binary() -> Path:
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / _NAME
        if candidate.is_file():
            return candidate
    return REPO_ROOT / "target" / "release" / _NAME


def main() -> int:
    project = Path(sys.argv[1])
    sample = int(sys.argv[2]) if len(sys.argv) > 2 else 40
    conn = sqlite3.connect(project / ".devzymer" / "index.sqlite3")

    rows = conn.execute(
        "select s.name, s.qualified_name, s.start_line, s.end_line, f.path "
        "from symbols s join files f on f.id = s.file_id "
        "where s.kind in ('function','method','class','interface','type') "
        "and f.path like '%.ts' order by f.path, s.start_line"
    ).fetchall()

    files: dict[str, list] = {}
    for name, qualified, start, end, path in rows:
        files.setdefault(path, []).append((name, qualified, start, end))

    checked = 0
    stale = []
    for path, symbols in files.items():
        if checked >= sample:
            break
        source = project / path
        if not source.is_file():
            continue
        lines = source.read_text(encoding="utf-8", errors="replace").splitlines()
        for name, qualified, start, end in symbols:
            if checked >= sample:
                break
            checked += 1
            window = "\n".join(lines[max(0, start - 2) : min(len(lines), end + 1)])
            if name not in window:
                stale.append((path, qualified, start, end))

    print(f"simbolos conferidos contra o disco: {checked}")
    print(f"referencias obsoletas: {len(stale)}")
    for item in stale[:10]:
        print("   ", item)
    return 0 if not stale else 1


if __name__ == "__main__":
    raise SystemExit(main())
