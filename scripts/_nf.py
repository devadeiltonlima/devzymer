"""Probes the MCP server of the given workspaces and prints the first line of each answer.

Usage: python scripts/_nf.py <projeto> [<projeto> ...]
The binary comes from DEVZYMER_BIN, or from the release build in this repo.
"""

import json
import os
import pathlib
import subprocess
import sys

here = pathlib.Path(__file__).resolve().parent.parent
override = os.environ.get("DEVZYMER_BIN")
if override:
    exe = pathlib.Path(override)
else:
    exe = here / "target" / "release" / ("devzymer.exe" if os.name == "nt" else "devzymer")

queries = [
    "app",
    "context_resolve",
    "como enviar uma mensagem para o usuario",
]

projects = [pathlib.Path(arg) for arg in sys.argv[1:]] if len(sys.argv) > 1 else []
if not projects:
    print("informe ao menos um projeto: python scripts/_nf.py <projeto> [...]")
    raise SystemExit(1)

for root in projects:
    for query in queries:
        payload = {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "context_resolve",
                "arguments": {"query": query},
            },
        }
        proc = subprocess.run(
            [str(exe), "serve", "--root", str(root)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
        )
        try:
            data = json.loads(proc.stdout.strip().splitlines()[0])
            text = data["result"]["content"][0]["text"]
            is_error = data["result"].get("isError", False)
        except Exception as exc:  # noqa: BLE001
            text = f"falha ao ler: {exc} | stdout={proc.stdout[:200]} stderr={proc.stderr[:200]}"
            is_error = True
        first = text.splitlines()[0] if text else ""
        print(f"[{root.name}] {query!r} erro={is_error}")
        print(f"   {first[:200]}")
        if is_error and "SYMBOL_NOT_FOUND" in first:
            print("   mensagem completa:")
            for line in text.splitlines()[:12]:
                print(f"     | {line[:170]}")
    print()
