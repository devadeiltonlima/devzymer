"""End to end check of the automatic synchronization.

Starts the real MCP server, reproduces the reported failure, changes a source
file and verifies that the change becomes searchable without any manual
sync. The original file content is restored and verified at the end.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import time
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
TARGET_FILE = "src/hooks/useVoiceCall.ts"
PROBE = "useVoiceCall"
NEW_SYMBOL = "useVoiceCallProbeSincronizacao"


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


def text_of(response: dict) -> str:
    return response["result"]["content"][0]["text"]


def symbols_of(response: dict) -> list[str]:
    """Names of the symbols actually returned, ignoring the error message."""
    structured = response.get("result", {}).get("structuredContent", {})
    return [item.get("qualified_name", "") for item in structured.get("results", [])]


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO_ROOT
    target = root / TARGET_FILE
    if not target.exists():
        print(f"arquivo de teste ausente: {target}")
        return 2

    original = target.read_text(encoding="utf-8")
    original_hash = hashlib.sha256(original.encode("utf-8")).hexdigest()
    failures: list[str] = []

    client = McpClient(root)
    try:
        status = client.call("context_status", {})
        status_text = text_of(status)
        print(f"context_status: {status_text.splitlines()[0]}")
        freshness = status["result"]["structuredContent"].get("freshness", {})
        if "pending_changes" not in freshness:
            failures.append("context_status does not expose the freshness block")
        watcher = status["result"]["structuredContent"].get("watcher") or {}
        if not watcher.get("running"):
            failures.append("the watcher is not running")

        tools = client.call("context_status", {})
        print("--- reproduction of the reported lookup ---")
        resolved = client.call("context_resolve", {"query": PROBE})
        resolved_text = text_of(resolved)
        print(resolved_text.splitlines()[0] if resolved_text.strip() else "(vazio)")
        if resolved.get("result", {}).get("isError"):
            failures.append(f"context_resolve('{PROBE}') failed: {resolved_text}")
        if TARGET_FILE not in resolved_text:
            failures.append(
                f"context_resolve('{PROBE}') did not return {TARGET_FILE}"
            )

        print("--- change the source file ---")
        target.write_text(
            original
            + f"\nexport function {NEW_SYMBOL}() {{\n  return true;\n}}\n",
            encoding="utf-8",
        )

        found = ""
        deadline = time.time() + 25
        while time.time() < deadline:
            probe = client.call("context_resolve", {"query": NEW_SYMBOL})
            probe_text = text_of(probe)
            if any(NEW_SYMBOL in name for name in symbols_of(probe)):
                found = probe_text
                break
            time.sleep(0.6)

        if not found:
            failures.append(
                f"the new symbol {NEW_SYMBOL} did not become searchable automatically"
            )
        else:
            print(f"new symbol found without a manual sync:\n{found.splitlines()[0]}")

        print("--- restore the original file ---")
        target.write_text(original, encoding="utf-8")
        restored_hash = hashlib.sha256(target.read_text(encoding="utf-8").encode("utf-8")).hexdigest()
        if restored_hash != original_hash:
            failures.append("the original file was not restored")

        gone = False
        deadline = time.time() + 25
        while time.time() < deadline:
            probe = client.call("context_resolve", {"query": NEW_SYMBOL})
            if not any(NEW_SYMBOL in name for name in symbols_of(probe)):
                gone = True
                break
            time.sleep(0.6)
        if not gone:
            failures.append("the removed symbol is still in the index")

        print("--- explicit synchronization ---")
        synced = client.call("context_sync", {})
        synced_text = text_of(synced)
        print(f"context_sync: {synced_text}")
        if not synced_text.startswith("OK"):
            failures.append("context_sync did not report success")
    finally:
        client.close()
        target.write_text(original, encoding="utf-8")
        final = hashlib.sha256(target.read_text(encoding="utf-8").encode("utf-8")).hexdigest()
        if final != original_hash:
            print("ERRO: o arquivo original nao foi restaurado")

    if failures:
        print("\nFALHAS:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nOK: a mudanca virou pesquisavel sozinha e o arquivo foi restaurado.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
