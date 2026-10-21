"""Regressao do P0-1/P0-3: resolucao de workspace fail-closed."""

import json
import os
import pathlib
import tempfile
import shutil
import subprocess
import sqlite3
import sys

EXE = pathlib.Path(
    os.environ.get(
        "DEVZYMER_BIN",
        str(pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "devzymer.exe"),
    )
)
SANDBOX = pathlib.Path(tempfile.gettempdir()) / "dz-regress"
HOME_INDEX = pathlib.Path.home() / ".devzymer" / "index.sqlite3"

results = []


def check(label, condition, detail=""):
    print(f"  [{'ok' if condition else 'FALHA'}] {label} {detail}")
    results.append((label, condition))
    return condition


def home_state():
    if not HOME_INDEX.exists():
        return None
    stat = HOME_INDEX.stat()
    wal = HOME_INDEX.with_name(HOME_INDEX.name + "-wal")
    return (
        stat.st_size,
        stat.st_mtime_ns,
        wal.stat().st_size if wal.exists() else 0,
        wal.stat().st_mtime_ns if wal.exists() else 0,
    )


def run(args, cwd=None):
    return subprocess.run(
        [str(EXE)] + args,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        cwd=str(cwd) if cwd else None,
        timeout=900,
    )


print("=== estado da home ANTES ===")
before = home_state()
print(f"  {before}")

if SANDBOX.exists():
    shutil.rmtree(SANDBOX, ignore_errors=True)

# --- caso 1: raiz explicita sem marcador (o que escalava para a home) ---
plain = SANDBOX / "sem-marcador"
plain.mkdir(parents=True)
(plain / "modulo.ts").write_text("export function alvo(){return 1}\n", encoding="utf-8")

print()
print("=== 1) init --root <dir sem marcador> ===")
proc = run(["init", "--agents", "claude", "--root", str(plain)])
out = (proc.stdout + proc.stderr)
print("   " + out.strip().splitlines()[0][:120] if out.strip() else "   (sem saida)")
check("exit 0", proc.returncode == 0, f"(exit {proc.returncode})")
check(
    "indice criado no diretorio pedido",
    (plain / ".devzymer" / "index.sqlite3").exists(),
)
check("a home NAO foi tocada", home_state() == before, f"{home_state()}")

# --- caso 2: status com raiz explicita sem indice ---
print()
print("=== 2) status --root <dir sem marcador e sem indice> ===")
vazio = SANDBOX / "vazio"
vazio.mkdir(parents=True)
proc = run(["status", "--root", str(vazio)])
out = (proc.stdout + proc.stderr)
check(
    "relata indice ausente no diretorio pedido",
    "n\u00e3o inicializado" in out or "n\u00e3o encontrado" in out,
)
check("aponta o diretorio pedido", str(vazio) in out, "")
check("a home NAO aparece na saida", str(HOME_INDEX.parent) not in out)

# --- caso 3: discover a partir de um diretorio sem marcador ---
print()
print("=== 3) status sem --root, em diretorio sem marcador ===")
proc = run(["status"], cwd=vazio)
out = (proc.stdout + proc.stderr)
check("falha explicita", proc.returncode != 0, f"(exit {proc.returncode})")
check("a home NAO aparece na saida", str(pathlib.Path.home() / ".devzymer") not in out)

# --- caso 4: o servidor MCP nao pode servir a home ---
print()
print("=== 4) serve --root <dir sem marcador>, cwd fora de projeto ===")
payload = json.dumps(
    {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "context_status", "arguments": {}},
    }
)
proc = subprocess.run(
    [str(EXE), "serve", "--root", str(vazio)],
    input=payload + "\n",
    capture_output=True,
    text=True,
    encoding="utf-8",
    errors="replace",
    cwd=str(vazio),
    timeout=300,
)
out = (proc.stdout + proc.stderr)
check("nao respondeu com a home", str(pathlib.Path.home() / ".devzymer") not in out)
check(
    "nao respondeu com simbolos de outro projeto",
    str(pathlib.Path(__file__).resolve().parent.parent) not in out,
)
print("   " + out.strip().splitlines()[-1][:200] if out.strip() else "   (sem saida)")

# --- caso 5: projeto legitimo continua funcionando ---
print()
print("=== 5) projeto legitimo (com marcador) continua operando ===")
proj = SANDBOX / "projeto"
(proj / "src").mkdir(parents=True)
(proj / "package.json").write_text('{"name":"p"}', encoding="utf-8")
(proj / "src" / "a.ts").write_text("export function alvo(){return 1}\n", encoding="utf-8")
proc = run(["init", "--agents", "claude", "--root", str(proj)])
check("init ok", proc.returncode == 0, f"(exit {proc.returncode})")
proc = run(["resolve", "alvo", "--root", str(proj), "--json"])
check(
    "resolve encontra o simbolo",
    proc.returncode == 0 and "alvo" in proc.stdout,
)

# --- caso 6: subdirectory resolve para a raiz do projeto ---
print()
print("=== 6) resolve a partir de um subdiretorio do projeto ===")
proc = run(["resolve", "alvo", "--root", str(proj / "src"), "--json"])
out = (proc.stdout + proc.stderr)
check(
    "raiz explicita nunca sobe para o ancestral",
    proc.returncode != 0,
    f"(exit {proc.returncode})",
)
check(
    "a mensagem aponta o projeto que tem indice",
    str(proj) in out,
)
check(
    "nenhum indice foi criado no subdiretorio",
    not (proj / "src" / ".devzymer").exists(),
)

# --- caso 7: o indice criado tem a identidade do workspace ---
print()
print("=== 7) identidade do workspace gravada no indice ===")
db = proj / ".devzymer" / "index.sqlite3"
con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
rows = dict(con.execute("SELECT key, value FROM meta").fetchall())
con.close()
check("meta tem workspace_id", "workspace_id" in rows, f"({list(rows.keys())})")

# --- caso 8: o indice nao pode ser servido a partir de outro workspace ---
print()
print("=== 8) indice copiado para outro projeto e recusado ===")
outro = SANDBOX / "outro-projeto"
(outro / "src").mkdir(parents=True)
(outro / "package.json").write_text('{"name":"o"}', encoding="utf-8")
(outro / "src" / "b.ts").write_text("export function outro(){return 2}\n", encoding="utf-8")
(outro / ".devzymer").mkdir(parents=True)
shutil.copy2(db, outro / ".devzymer" / "index.sqlite3")
proc = run(["resolve", "alvo", "--root", str(outro), "--json"])
out = (proc.stdout + proc.stderr)
check(
    "indice de outro workspace e recusado",
    proc.returncode != 0 and "WORKSPACE_MISMATCH" in out,
    f"(exit {proc.returncode})",
)
check("nao devolve simbolos do indice alheio", "alvo" not in out)

print()
print("=== estado da home DEPOIS ===")
after = home_state()
print(f"  {after}")
check("a home permaneceu intocada", before == after)

print()
falhas = [name for name, ok in results if not ok]
print(f"resultado: {len(results) - len(falhas)}/{len(results)} ok")
for name in falhas:
    print("  FALHA:", name)
sys.exit(1 if falhas else 0)
