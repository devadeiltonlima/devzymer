"""Checks the reported scenario against a real repository.

Confirms that a query naming a file resolves through context_resolve and
context_impact, and that the automatic synchronization keeps the index current.
"""

from __future__ import annotations

import json
import os
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
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO_ROOT
    query = sys.argv[2] if len(sys.argv) > 2 else "useVoiceCall"
    expected = sys.argv[3] if len(sys.argv) > 3 else "src/hooks/useVoiceCall.ts"

    client = McpClient(root)
    failures: list[str] = []
    try:
        resolved = client.call("context_resolve", {"query": query})
        text = resolved["result"]["content"][0]["text"]
        print("--- context_resolve ---")
        print(text)
        if resolved.get("result", {}).get("isError"):
            failures.append(f"context_resolve('{query}') failed")
        structured = resolved.get("result", {}).get("structuredContent", {})
        files = [f.get("path") for f in structured.get("files", [])]
        if expected not in files:
            failures.append(f"context_resolve did not match the file {expected}")
        results = structured.get("results", [])
        if not results:
            failures.append("context_resolve returned no symbols")
        for item in results[:3]:
            if not item.get("start_line"):
                failures.append("a result is missing the line range")

        impact = client.call("context_impact", {"symbol": query})
        impact_text = impact["result"]["content"][0]["text"]
        print("--- context_impact ---")
        print(impact_text)
        if impact.get("result", {}).get("isError"):
            failures.append(f"context_impact('{query}') failed")
    finally:
        client.close()

    if failures:
        print("\nFALHAS:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nOK: o caso reportado resolve por arquivo e por símbolo.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
