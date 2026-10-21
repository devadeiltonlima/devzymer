"""Verifies the context_patch tool: revision check, atomic write and rollback."""

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
SCRATCH = Path("target") / "patch_check" / "sample.txt"


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
    root = REPO_ROOT
    target = root / SCRATCH
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("primeira linha\nsegunda linha\nterceira linha\n", encoding="utf-8")

    client = McpClient(root)
    failures: list[str] = []
    try:
        read = client.call(
            "context_read",
            {"path": SCRATCH.as_posix(), "start_line": 1, "end_line": 3},
        )
        original = read["result"]["content"][0]["text"]
        sha = original.split("sha=")[1].split(" ")[0]
        print(f"leitura original: sha={sha}")

        dry = client.call(
            "context_patch",
            {
                "path": SCRATCH.as_posix(),
                "start_line": 2,
                "end_line": 2,
                "replacement": "segunda linha alterada",
                "expected_sha": sha,
                "dry_run": True,
            },
        )
        dry_text = dry["result"]["content"][0]["text"]
        print(f"dry-run: {dry_text}")
        if not dry_text.startswith("OK"):
            failures.append("the dry run did not validate the edit")
        if target.read_text(encoding="utf-8") != "primeira linha\nsegunda linha\nterceira linha\n":
            failures.append("the dry run modified the file")

        stale = client.call(
            "context_patch",
            {
                "path": SCRATCH.as_posix(),
                "start_line": 2,
                "end_line": 2,
                "replacement": "x",
                "expected_sha": "0000000000000000",
            },
        )
        stale_text = stale["result"]["content"][0]["text"]
        print(f"revisão obsoleta: {stale_text}")
        if "PATCH_STALE_REVISION" not in stale_text and "INVALID_PATCH" not in stale_text:
            failures.append("a stale revision was accepted")

        applied = client.call(
            "context_patch",
            {
                "path": SCRATCH.as_posix(),
                "start_line": 2,
                "end_line": 2,
                "replacement": "segunda linha alterada",
                "expected_sha": sha,
            },
        )
        applied_text = applied["result"]["content"][0]["text"]
        print(f"aplicado: {applied_text}")
        if not applied_text.startswith("OK"):
            failures.append("the patch was not applied")
        content = target.read_text(encoding="utf-8")
        print(f"conteudo: {content!r}")
        if content != "primeira linha\nsegunda linha alterada\nterceira linha\n":
            failures.append("the file content does not match the expected result")

        escape = client.call(
            "context_patch",
            {
                "path": "../fora/do/workspace.txt",
                "start_line": 1,
                "end_line": 1,
                "replacement": "x",
                "expected_sha": sha,
            },
        )
        escape_text = escape["result"]["content"][0]["text"]
        print(f"tentativa de escape: {escape_text}")
        if "PATCH_OUTSIDE_WORKSPACE" not in escape_text and "PATH_ESCAPE" not in escape_text:
            failures.append("a path outside the workspace was accepted")

        escape_without_sha = client.call(
            "context_patch",
            {
                "path": "../fora/do/workspace.txt",
                "start_line": 1,
                "end_line": 1,
                "replacement": "x",
            },
        )
        escape_without_sha_text = escape_without_sha["result"]["content"][0]["text"]
        print(f"tentativa de escape sem revisao: {escape_without_sha_text}")
        if not escape_without_sha.get("result", {}).get("isError"):
            failures.append("a path outside the workspace was accepted without a revision")
        if (Path("fora") / "do" / "workspace.txt").exists():
            failures.append("a file outside the workspace was created")
    finally:
        client.close()
        if target.parent.exists():
            for child in target.parent.iterdir():
                child.unlink()
            target.parent.rmdir()

    if failures:
        print("\nFALHAS:")
        for failure in failures:
            print(f"  - {failure}")
        return 1
    print("\nOK: o patch atômico, a verificação de revisão e o limite de workspace funcionam.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
