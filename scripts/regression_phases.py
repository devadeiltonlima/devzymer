"""Regressao das fases 4 a 8: privacidade, saida limitada, manutencao, ranking e benchmark."""

import json
import os
import pathlib
import tempfile
import re
import shutil
import subprocess
import sys

EXE = pathlib.Path(
    os.environ.get(
        "DEVZYMER_BIN",
        str(pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "devzymer.exe"),
    )
)
SANDBOX = pathlib.Path(tempfile.gettempdir()) / "dz-fases"
results = []


def check(label, condition, detail=""):
    print(f"  [{'ok' if condition else 'FALHA'}] {label} {detail}")
    results.append((label, condition))
    return condition


def run(args, cwd=None, timeout=900):
    return subprocess.run(
        [str(EXE)] + args,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=str(cwd) if cwd else None,
        timeout=timeout,
    )


def mcp(root, name, args):
    payload = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        }
    )
    proc = subprocess.run(
        [str(EXE), "serve", "--root", str(root)],
        input=payload + "\n",
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=300,
    )
    lines = [l for l in proc.stdout.strip().splitlines() if l.strip()]
    if not lines:
        return {"text": "", "code": "NO_RESPONSE"}
    data = json.loads(lines[-1])
    result = data.get("result", {})
    text = result.get("content", [{}])[0].get("text", "")
    code = text.split(":", 1)[0].replace("ERR ", "").strip() if text.startswith("ERR ") else "OK"
    return {"text": text, "code": code}


if SANDBOX.exists():
    shutil.rmtree(SANDBOX, ignore_errors=True)

root = SANDBOX / "projeto"
(root / "src").mkdir(parents=True)
(root / "package.json").write_text('{"name":"p"}', encoding="utf-8")
(root / "src" / "app.ts").write_text("export function app(){return 1}\n", encoding="utf-8")

# --- fase 4: privacidade ---
print("=== fase 4: arquivos sensiveis nao sao indexados ===")
(root / ".env").write_text("AWS_SECRET_ACCESS_KEY=AKIAIOSFODNN7EXAMPLE\n", encoding="utf-8")
(root / "credentials.json").write_text('{"token":"ghp_abcdefghijklmnopqrstuvwxyz012345"}\n', encoding="utf-8")
(root / "server.key").write_text("-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n", encoding="utf-8")
(root / "src" / "com_segredo.ts").write_text(
    'const chave = "ghp_abcdefghijklmnopqrstuvwxyz012345";\nexport function f(){return 1}\n',
    encoding="utf-8",
)
(root / "src" / "normal.ts").write_text("export function normal(){return 2}\n", encoding="utf-8")

proc = run(["init", "--agents", "claude", "--root", str(root)])
check("init ok", proc.returncode == 0, f"(exit {proc.returncode})")

import sqlite3

db = root / ".devzymer" / "index.sqlite3"
con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
paths = [r[0] for r in con.execute("SELECT path FROM files").fetchall()]
con.close()
check(".env nao indexado", not any(".env" in p for p in paths), f"({paths})")
check("credentials.json nao indexado", not any("credentials" in p for p in paths))
check("server.key nao indexado", not any("server.key" in p for p in paths))
check(
    "arquivo com segredo no conteudo nao indexado",
    not any("com_segredo" in p for p in paths),
    f"({paths})",
)
check("arquivo comum indexado", any("normal.ts" in p for p in paths))

# --- fase 6: saida limitada ---
print()
print("=== fase 6: a resposta nao pode passar do teto ===")
grande = root / "src" / "grande.txt"
grande.write_text("linha de teste\n" * 20000, encoding="utf-8")
run(["sync", "--root", str(root)])
res = mcp(root, "context_read", {"path": "src/grande.txt", "max_bytes": 10**9})
tamanho = len(res["text"])
check("resposta abaixo de 256 KiB", tamanho < 256 * 1024, f"({tamanho} chars)")
check("resposta nao vazia", tamanho > 0)

# --- fase 5: manutencao ---
print()
print("=== fase 5: manutencao do indice ===")
proc = run(["maintain", "--root", str(root), "--vacuum"])
check("maintain ok", proc.returncode == 0, f"(exit {proc.returncode})")
check("relata o tamanho", "tamanho:" in proc.stdout)
check("relata o checkpoint", "checkpoint:" in proc.stdout)
proc = run(["maintain", "--root", str(root), "--json"])
check("maintain --json ok", proc.returncode == 0)
try:
    payload = json.loads(proc.stdout)
    check("json tem as metricas", "wal_bytes_before" in payload and "free_pages_after" in payload)
except Exception as exc:
    check("json tem as metricas", False, f"({exc})")

# --- fase 7: ranking e ambiguidade ---
print()
print("=== fase 7: a busca nao inunda com membros do arquivo ===")
proj = root
externo = os.environ.get("DEVZYMER_TEST_PROJECT")
if externo:
    candidato = pathlib.Path(externo)
    if (candidato / ".devzymer" / "index.sqlite3").exists():
        proj = candidato
if (proj / ".devzymer" / "index.sqlite3").exists():
    res = mcp(proj, "context_resolve", {"query": "app"})
    linhas = [l for l in res["text"].splitlines() if l.strip()]
    por_arquivo = {}
    for linha in linhas:
        arquivo = linha.split(":", 1)[0]
        por_arquivo[arquivo] = por_arquivo.get(arquivo, 0) + 1
    maior = max(por_arquivo.values()) if por_arquivo else 0
    check(
        "no maximo 6 candidatos por arquivo",
        maior <= 6,
        f"({maior} candidatos em um arquivo)",
    )
    res = mcp(proj, "context_resolve", {"query": "app"})
    check(
        "ambiguidade falsa eliminada para simbolo exportado",
        "ambiguidade" not in res["text"],
        f"({res['text'].splitlines()[0][:70]})",
    )
else:
    print("   (projeto de teste sem indice, etapa ignorada)")

# --- fase 8: benchmark ---
print()
print("=== fase 8: o benchmark mede o que diz ===")
if (proj / ".devzymer" / "index.sqlite3").exists():
    seeds = {}
    for seed in ("777", "20260319"):
        proc = run(["benchmark", "--root", str(proj), "--tasks", "20", "--seed", seed, "--format", "json"])
        if proc.returncode == 0:
            data = json.loads(proc.stdout)
            ids = [
                ((r.get("task") or {}).get("query"), (r.get("task") or {}).get("kind"))
                for r in data.get("results", [])
            ]
            seeds[seed] = ids
            check(f"seed {seed} produz tarefas", len(ids) > 0, f"({len(ids)})")
    if len(seeds) == 2:
        check(
            "seeds diferentes produzem conjuntos diferentes",
            seeds["777"] != seeds["20260319"],
            "",
        )
    proc = run(["benchmark", "--root", str(proj), "--tasks", "20", "--seed", "20260319", "--format", "json"])
    data = json.loads(proc.stdout)
    cenarios = {k.get("kind") for k in data.get("by_kind", []) if isinstance(k, dict)}
    check("o cenario filename_discovery aparece", "filename_discovery" in cenarios, f"({sorted(cenarios)})")
    check(
        "a metrica do cache nao e inventada",
        data.get("cache", {}).get("token_saving_from_cache_pct") in (None, 0.0),
    )

print()
falhas = [n for n, ok in results if not ok]
print(f"resultado: {len(results) - len(falhas)}/{len(results)} ok")
for n in falhas:
    print("  FALHA:", n)
sys.exit(1 if falhas else 0)
