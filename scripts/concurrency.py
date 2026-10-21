"""Verifica que vários processos MCP compartilham o mesmo índice sem corromper."""

import json
import pathlib
import tempfile
import subprocess
import sys
import threading

EXE = pathlib.Path(
    str(pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "devzymer.exe")
)
ROOT = pathlib.Path(tempfile.gettempdir()) / "dz-concorrencia"

if ROOT.exists():
    import shutil

    shutil.rmtree(ROOT, ignore_errors=True)
(ROOT / "src").mkdir(parents=True, exist_ok=True)
(ROOT / "package.json").write_text('{"name":"demo"}', encoding="utf-8")
for index in range(40):
    (ROOT / "src" / f"modulo{index}.ts").write_text(
        f"export function funcao{index}() {{ return {index}; }}\n", encoding="utf-8"
    )

subprocess.run(
    [str(EXE), "init", "--agents", "claude", "--root", str(ROOT)],
    capture_output=True,
    text=True,
    check=True,
)

errors = []
results = []


def consult(index):
    payload = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "context_resolve",
                "arguments": {"query": f"funcao{index}"},
            },
        }
    )
    try:
        proc = subprocess.run(
            [str(EXE), "serve", "--root", str(ROOT)],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=120,
        )
        data = json.loads(proc.stdout.strip().splitlines()[0])
        text = data["result"]["content"][0]["text"]
        if f"funcao{index}" not in text:
            errors.append(f"processo {index}: resultado sem o simbolo esperado")
        else:
            results.append(index)
    except Exception as exc:  # noqa: BLE001
        errors.append(f"processo {index}: {exc}")


threads = [threading.Thread(target=consult, args=(index,)) for index in range(40)]
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()

print(f"consultas concorrentes: {len(results)}/40 com o simbolo correto")
if errors:
    for error in errors[:5]:
        print("  erro:", error)

integrity = subprocess.run(
    [str(EXE), "doctor", "--root", str(ROOT)],
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    encoding="utf-8",
    errors="replace",
)
print("integridade apos a concorrencia:", "ok" if "[ok] banco" in integrity.stdout else "FALHA")
for line in integrity.stdout.splitlines():
    if "banco" in line or "fts5" in line:
        print("  ", line)

sys.exit(1 if errors or "[ok] banco" not in integrity.stdout else 0)
