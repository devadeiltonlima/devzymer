"""Regressao do P0-2: integridade do patch via MCP real."""

import json
import os
import pathlib
import tempfile
import shutil
import subprocess
import sys

EXE = pathlib.Path(
    os.environ.get(
        "DEVZYMER_BIN",
        str(pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "devzymer.exe"),
    )
)
SANDBOX = pathlib.Path(tempfile.gettempdir()) / "dz-patch"

results = []


def check(label, condition, detail=""):
    print(f"  [{'ok' if condition else 'FALHA'}] {label} {detail}")
    results.append((label, condition))
    return condition


def mcp(root, name, args, cwd=None):
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
        cwd=str(cwd) if cwd else None,
        timeout=300,
    )
    lines = [l for l in proc.stdout.strip().splitlines() if l.strip()]
    if not lines:
        return {"error": (proc.stderr or "").strip()[:300], "code": "NO_RESPONSE"}
    data = json.loads(lines[-1])
    result = data.get("result", {})
    text = result.get("content", [{}])[0].get("text", "")
    code = "OK"
    if text.startswith("ERR "):
        code = text.split(":", 1)[0].replace("ERR ", "").strip()
    return {"text": text, "code": code, "structured": result.get("structuredContent")}


if SANDBOX.exists():
    shutil.rmtree(SANDBOX, ignore_errors=True)

root = SANDBOX / "projeto"
(root / "src").mkdir(parents=True)
(root / "package.json").write_text('{"name":"p"}', encoding="utf-8")
(root / "src" / "mod.ts").write_text(
    "export function alvo() {\n" + "".join(f"  const p{i} = {i};\n" for i in range(98)) + "  return 1;\n}\n",
    encoding="utf-8",
)
(root / "src" / "crlf.txt").write_bytes(b"alpha\r\nbeta\r\ngamma\r\n")
(root / "src" / "utf8.txt").write_bytes("ação\nsegunda\n".encode("utf-8"))

subprocess.run(
    [str(EXE), "init", "--agents", "claude", "--root", str(root)],
    capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900,
)

print("=== 1) substituicao correta de uma funcao ===")
read = mcp(root, "context_read", {"symbol": "alvo", "path": "src/mod.ts"})
text = read["text"]
print("   " + text.splitlines()[0][:110])
import re

sha = re.search(r"sha=([0-9a-f]{16})", text)
sha = sha.group(1) if sha else None
check("a leitura devolve um sha", sha is not None)
res = mcp(
    root,
    "context_patch",
    {
        "path": "src/mod.ts",
        "start_line": 1,
        "end_line": 2,
        "replacement": "export function alvo() {\n  const nova = 1;",
        "expected_sha": sha,
    },
)
check("patch aceito", res["code"] == "OK", f"({res['code']})")
content = (root / "src" / "mod.ts").read_text(encoding="utf-8")
check("a funcao foi alterada", "const nova = 1;" in content)
check("o resto do arquivo continua", content.count("const p") == 97)

print()
print("=== 2) linha inicial incorreta (fora do arquivo) ===")
before = (root / "src" / "mod.ts").read_bytes()
read = mcp(root, "context_read", {"symbol": "alvo", "path": "src/mod.ts"})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
res = mcp(
    root,
    "context_patch",
    {"path": "src/mod.ts", "start_line": 9000, "end_line": 9001, "replacement": "x", "expected_sha": sha},
)
check("rejeitado", res["code"] != "OK", f"({res['code']})")
check("arquivo intacto", (root / "src" / "mod.ts").read_bytes() == before)

print()
print("=== 3) linha final incorreta (maior que o arquivo) ===")
res = mcp(
    root,
    "context_patch",
    {"path": "src/mod.ts", "start_line": 1, "end_line": 500, "replacement": "x", "expected_sha": sha},
)
check("rejeitado", res["code"] != "OK", f"({res['code']})")
check("arquivo intacto", (root / "src" / "mod.ts").read_bytes() == before)

print()
print("=== 4) hash obsoleto (arquivo mudou depois da leitura) ===")
res = mcp(
    root,
    "context_patch",
    {"path": "src/mod.ts", "start_line": 1, "end_line": 2, "replacement": "x", "expected_sha": "0123456789abcdef"},
)
check("rejeitado com PATCH_STALE_REVISION", res["code"] == "PATCH_STALE_REVISION", f"({res['code']})")
check("arquivo intacto", (root / "src" / "mod.ts").read_bytes() == before)

print()
print("=== 5) modificacao concorrente ===")
read = mcp(root, "context_read", {"symbol": "alvo", "path": "src/mod.ts"})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
with (root / "src" / "mod.ts").open("a", encoding="utf-8") as fh:
    fh.write("// alterado por outro processo\n")
after_concurrent = (root / "src" / "mod.ts").read_bytes()
res = mcp(
    root,
    "context_patch",
    {"path": "src/mod.ts", "start_line": 1, "end_line": 2, "replacement": "x", "expected_sha": sha},
)
check("rejeitado", res["code"] == "PATCH_STALE_REVISION", f"({res['code']})")
check("a alteracao concorrente nao foi perdida", (root / "src" / "mod.ts").read_bytes() == after_concurrent)

print()
print("=== 6) conteudo nao ASCII ===")
read = mcp(root, "context_read", {"path": "src/utf8.txt", "start_line": 1, "end_line": 2})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
res = mcp(
    root,
    "context_patch",
    {"path": "src/utf8.txt", "start_line": 2, "end_line": 2, "replacement": "alterada", "expected_sha": sha},
)
check("patch aceito", res["code"] == "OK", f"({res['code']})")
check(
    "bytes fora do intervalo preservados",
    (root / "src" / "utf8.txt").read_bytes() == "ação\nalterada\n".encode("utf-8"),
)

print()
print("=== 7) CRLF preservado ===")
read = mcp(root, "context_read", {"path": "src/crlf.txt", "start_line": 1, "end_line": 3})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
res = mcp(
    root,
    "context_patch",
    {"path": "src/crlf.txt", "start_line": 2, "end_line": 2, "replacement": "BETA", "expected_sha": sha},
)
check("patch aceito", res["code"] == "OK", f"({res['code']})")
check(
    "CRLF preservado",
    (root / "src" / "crlf.txt").read_bytes() == b"alpha\r\nBETA\r\ngamma\r\n",
)

print()
print("=== 8) substituicao vazia (remocao de um bloco) ===")
read = mcp(root, "context_read", {"path": "src/crlf.txt", "start_line": 1, "end_line": 3})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
res = mcp(
    root,
    "context_patch",
    {"path": "src/crlf.txt", "start_line": 3, "end_line": 3, "replacement": "", "expected_sha": sha},
)
check("patch aceito", res["code"] == "OK", f"({res['code']})")
check(
    "apenas a linha alvo saiu",
    (root / "src" / "crlf.txt").read_bytes() == b"alpha\r\nBETA\r\n",
)

print()
print("=== 9) exclusao acidental do arquivo inteiro ===")
read = mcp(root, "context_read", {"path": "src/crlf.txt", "start_line": 1, "end_line": 2})
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
before = (root / "src" / "crlf.txt").read_bytes()
res = mcp(
    root,
    "context_patch",
    {"path": "src/crlf.txt", "start_line": 1, "end_line": 2, "replacement": "nada", "expected_sha": sha},
)
check("rejeitado", res["code"] != "OK", f"({res['code']})")
check("arquivo intacto", (root / "src" / "crlf.txt").read_bytes() == before)

print()
print("=== 10) escape por link simbolico/junction ===")
fora = SANDBOX / "fora"
fora.mkdir(parents=True)
(fora / "alvo.txt").write_text("segredo\n", encoding="utf-8")
link = root / "src" / "link.txt"
try:
    os.symlink(fora / "alvo.txt", link)
except OSError:
    subprocess.run(
        ["cmd", "/c", "mklink", "/H", str(link), str(fora / "alvo.txt")],
        capture_output=True,
        timeout=60,
    )
if link.exists():
    res = mcp(
        root,
        "context_patch",
        {"path": "src/link.txt", "start_line": 1, "end_line": 1, "replacement": "x", "expected_sha": "0" * 16},
    )
    check("escape rejeitado", res["code"] != "OK", f"({res['code']})")
    check("o arquivo de fora nao foi alterado", (fora / "alvo.txt").read_text() == "segredo\n")
else:
    print("   (link nao criado neste ambiente)")

print()
print("=== 11) edicao em outro workspace ===")
outro = SANDBOX / "outro"
(outro / "src").mkdir(parents=True)
(outro / "package.json").write_text('{"name":"o"}', encoding="utf-8")
(outro / "src" / "x.ts").write_text("export function x(){return 1}\n", encoding="utf-8")
subprocess.run(
    [str(EXE), "init", "--agents", "claude", "--root", str(outro)],
    capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900,
)
res = mcp(
    root,
    "context_patch",
    {"path": "../outro/src/x.ts", "start_line": 1, "end_line": 1, "replacement": "x", "expected_sha": "0" * 16},
)
check("rejeitado", res["code"] != "OK", f"({res['code']})")
check(
    "o outro projeto nao foi alterado",
    (outro / "src" / "x.ts").read_text() == "export function x(){return 1}\n",
)

print()
print("=== 12) fluxo resolve -> read -> patch (o defeito original) ===")
alvo = root / "src" / "grande.ts"
alvo.write_text(
    "export function grande() {\n" + "".join(f"  const q{i} = {i};\n" for i in range(98)) + "  return 1;\n}\n",
    encoding="utf-8",
)
subprocess.run(
    [str(EXE), "sync", "--root", str(root)], capture_output=True, text=True, timeout=900,
)
read = mcp(root, "context_read", {"symbol": "grande", "path": "src/grande.ts"})
header = read["text"].splitlines()[0]
print("   " + header[:120])
sha = re.search(r"sha=([0-9a-f]{16})", read["text"]).group(1)
linhas_antes = len(alvo.read_text(encoding="utf-8").splitlines())
res = mcp(
    root,
    "context_patch",
    {
        "path": "src/grande.ts",
        "start_line": 1,
        "end_line": 2,
        "replacement": "export function grande() {\n  const nova = 1;",
        "expected_sha": sha,
    },
)
linhas_depois = len(alvo.read_text(encoding="utf-8").splitlines())
check("patch aceito", res["code"] == "OK", f"({res['code']})")
check(
    "nenhuma linha foi apagada",
    linhas_depois == linhas_antes,
    f"({linhas_antes} -> {linhas_depois})",
)
check("o conteudo posterior permanece", alvo.read_text().count("const q") == 97)

print()
falhas = [n for n, ok in results if not ok]
print(f"resultado: {len(results) - len(falhas)}/{len(results)} ok")
for n in falhas:
    print("  FALHA:", n)
sys.exit(1 if falhas else 0)
