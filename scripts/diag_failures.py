"""Diagnostic for the failing retrieval cases."""

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
    override = os.environ.get("DEVZYMER_BIN")
    if override:
        return Path(override)
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / _NAME
        if candidate.is_file():
            return candidate
    return REPO_ROOT / "target" / "release" / _NAME


BINARY = _binary()


class McpClient:
    def __init__(self, root: Path) -> None:
        self.process = subprocess.Popen(
            [str(BINARY), "serve", "--root", str(root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.counter = 0

    def call(self, name: str, arguments: dict) -> dict:
        self.counter += 1
        message = {
            "jsonrpc": "2.0",
            "id": self.counter,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        assert self.process.stdout is not None
        return json.loads(self.process.stdout.readline())

    def close(self) -> None:
        try:
            if self.process.stdin:
                self.process.stdin.close()
            self.process.wait(timeout=10)
        except Exception:
            self.process.kill()


def main() -> int:
    root = Path(sys.argv[1])
    conn = sqlite3.connect(root / ".devzymer" / "index.sqlite3")

    for name in ("ferramentas", "objeto"):
        rows = conn.execute(
            "select s.qualified_name, s.kind, s.start_line, f.path from symbols s "
            "join files f on f.id = s.file_id where lower(s.name) = ? order by f.path, s.start_line",
            (name.lower(),),
        ).fetchall()
        print(f"=== simbolos com o nome {name!r}: {len(rows)} ===")
        for row in rows[:15]:
            print("   ", row)

    client = McpClient(root)
    try:
        for query in ("ferramentas", "objeto", "SampleService.ferramentas", "limites.ts objeto"):
            response = client.call("context_resolve", {"query": query, "limit": 10})
            structured = response.get("result", {}).get("structuredContent", {})
            print(f"\n=== consulta {query!r} ===")
            for index, item in enumerate(structured.get("results", []), start=1):
                print(
                    f"  {index:>2}. {item['path']}:{item['start_line']} | {item['qualified_name']} "
                    f"| {item['kind']} | {item['confidence']} | origem={item['source']} "
                    f"| score={item['score']:.3f} | termos={item.get('matched_terms')}"
                )
    finally:
        client.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
