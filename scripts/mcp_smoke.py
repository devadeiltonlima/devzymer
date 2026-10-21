"""Smoke test of the devzymer MCP server over stdio.

Spawns the server, performs the initialize handshake, lists the tools and
calls each one against a real indexed repository.
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
    local_target = Path(__import__("os").environ.get("LOCALAPPDATA", "")) / "devzymer-target" / "release" / _NAME
    if local_target.is_file():
        return local_target
    if os.environ.get("CARGO_TARGET_DIR"):
        shipped = Path(os.environ["CARGO_TARGET_DIR"]) / "release" / _NAME
        if shipped.is_file():
            return shipped
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

    def request(self, method: str, params: dict | None = None) -> dict:
        self.counter += 1
        message = {"jsonrpc": "2.0", "id": self.counter, "method": method}
        if params is not None:
            message["params"] = params
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        assert self.process.stdout is not None
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(f"the server closed the stream during {method}")
        return json.loads(line)

    def notify(self, method: str, params: dict | None = None) -> None:
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def close(self) -> None:
        try:
            if self.process.stdin:
                self.process.stdin.close()
            self.process.wait(timeout=10)
        except Exception:
            self.process.kill()


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else REPO_ROOT
    if not BINARY.exists():
        print(f"binary not found: {BINARY}")
        return 2

    client = McpClient(root)
    failures: list[str] = []

    try:
        initialize = client.request(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "devzymer-smoke", "version": "1.0.0"},
            },
        )
        result = initialize.get("result", {})
        print(f"initialize: protocolo={result.get('protocolVersion')} servidor={result.get('serverInfo')}")
        if "result" not in initialize:
            failures.append("initialize did not return a result")
        client.notify("notifications/initialized")

        tools = client.request("tools/list")
        names = [t["name"] for t in tools.get("result", {}).get("tools", [])]
        print(f"tools/list: {names}")
        expected = {
            "context_resolve",
            "context_read",
            "context_impact",
            "context_patch",
            "context_status",
            "context_sync",
            "context_decide",
        }
        missing = expected - set(names)
        if missing:
            failures.append(f"missing tools: {sorted(missing)}")

        status = client.request("tools/call", {"name": "context_status", "arguments": {}})
        status_text = status["result"]["content"][0]["text"]
        print(f"context_status: {status_text}")
        structured = status["result"].get("structuredContent", {})
        freshness = structured.get("freshness", {})
        for field in ("last_sync_ms", "pending_changes", "skipped", "failures"):
            if field not in freshness:
                failures.append(f"context_status does not expose freshness.{field}")
        if "watcher" not in structured:
            failures.append("context_status does not expose the watcher state")

        synced = client.request("tools/call", {"name": "context_sync", "arguments": {}})
        synced_text = synced["result"]["content"][0]["text"]
        print(f"context_sync: {synced_text}")
        if not synced_text.startswith("OK"):
            failures.append("context_sync did not report success")

        resolve = client.request(
            "tools/call",
            {"name": "context_resolve", "arguments": {"query": "content_hash", "limit": 3}},
        )
        resolve_text = resolve["result"]["content"][0]["text"]
        print(f"context_resolve:\n{resolve_text}")
        if not resolve_text.strip():
            failures.append("context_resolve returned an empty payload")

        first_line = resolve_text.splitlines()[0] if resolve_text.strip() else ""
        path_part = first_line.split("|")[0].strip() if first_line else ""
        file_path = path_part.rsplit(":", 1)[0] if ":" in path_part else ""
        symbol_name = first_line.split("|")[1].strip() if "|" in first_line else ""
        if file_path:
            read = client.request(
                "tools/call",
                {
                    "name": "context_read",
                    "arguments": {
                        "path": file_path,
                        "symbol": symbol_name,
                        "max_bytes": 800,
                    },
                },
            )
            read_text = read["result"]["content"][0]["text"]
            print(f"context_read: {read_text.splitlines()[0] if read_text else '(vazio)'}")
            if not read_text.strip():
                failures.append("context_read returned an empty payload")
            if symbol_name and symbol_name not in read_text:
                failures.append("context_read did not return the requested symbol")

        impact = client.request(
            "tools/call",
            {"name": "context_impact", "arguments": {"path": "crates/devzymer-core/src/store.rs"}},
        )
        print(f"context_impact: {impact['result']['content'][0]['text'].splitlines()[0]}")

        decide = client.request(
            "tools/call",
            {
                "name": "context_decide",
                "arguments": {"task": "evidence_sufficient", "state": "exact_symbol confirmed"},
            },
        )
        print(f"context_decide: {decide['result']['content'][0]['text']}")

        # A well-formed search with zero matches is a success with an honest
        # payload, not a tool error (empty-search semantics of the MCP spec).
        unknown_symbol = client.request(
            "tools/call",
            {"name": "context_resolve", "arguments": {"query": "zzz_symbol_que_nao_existe_zzz"}},
        )
        unknown_result = unknown_symbol.get("result", {})
        unknown_text = (unknown_result.get("content") or [{}])[0].get("text", "")
        print(f"context_resolve (desconhecido): {unknown_text.splitlines()[0] if unknown_text else '(vazio)'}")
        if unknown_result.get("isError"):
            failures.append("context_resolve must not report a tool error for zero matches")
        if not unknown_text.startswith("nao_encontrado"):
            failures.append("context_resolve zero matches must answer honestly with nao_encontrado")
        if not (unknown_result.get("structuredContent") or {}).get("not_found"):
            failures.append("context_resolve zero matches must expose not_found=true")

        # The dry-run validates the edit, but SHA enforcement stands even in
        # dry-run, and a dry-run never writes.
        target_path = "crates/devzymer-core/src/store.rs"
        target_file = root / target_path
        digest_before = hashlib.sha256(target_file.read_bytes()).hexdigest()
        read = client.request(
            "tools/call",
            {"name": "context_read", "arguments": {"path": target_path, "max_bytes": 2000}},
        )
        read_state = (read.get("result", {}).get("structuredContent") or {})
        expected_sha = read_state.get("sha") or ""
        if not expected_sha:
            failures.append("context_read did not return the sha needed by context_patch")

        without_sha = client.request(
            "tools/call",
            {
                "name": "context_patch",
                "arguments": {
                    "path": target_path,
                    "start_line": 1,
                    "end_line": 1,
                    "replacement": "// nao deve ser aplicado\n",
                    "dry_run": True,
                },
            },
        )
        if not without_sha.get("result", {}).get("isError"):
            failures.append("context_patch without expected_sha must be rejected even in dry-run")

        with_wrong_sha = client.request(
            "tools/call",
            {
                "name": "context_patch",
                "arguments": {
                    "path": target_path,
                    "start_line": 1,
                    "end_line": 1,
                    "replacement": "// nao deve ser aplicado\n",
                    "dry_run": True,
                    "expected_sha": "0000000000000000",
                },
            },
        )
        if not with_wrong_sha.get("result", {}).get("isError"):
            failures.append("context_patch with a wrong expected_sha must be rejected")

        invalid_patch = client.request(
            "tools/call",
            {
                "name": "context_patch",
                "arguments": {
                    "path": target_path,
                    "start_line": 1,
                    "end_line": 1,
                    "replacement": "// nao deve ser aplicado\n",
                    "dry_run": True,
                    "expected_sha": expected_sha,
                },
            },
        )
        patch_text = invalid_patch["result"]["content"][0]["text"]
        print(f"context_patch (dry-run): {patch_text}")
        if not patch_text.startswith("OK"):
            failures.append("context_patch dry-run did not validate the edit")
        if not (invalid_patch.get("result", {}).get("structuredContent") or {}).get("dry_run"):
            failures.append("context_patch dry-run must expose dry_run=true")
        digest_after = hashlib.sha256(target_file.read_bytes()).hexdigest()
        if digest_after != digest_before:
            failures.append("context_patch dry-run wrote to the file")

    finally:
        client.close()

    if failures:
        print("\nFALHAS:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nOK: o servidor MCP respondeu conforme o protocolo.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
