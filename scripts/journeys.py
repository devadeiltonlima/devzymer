"""Verificação das jornadas de instalação multi-agente."""

import json
import pathlib
import tempfile
import shutil
import subprocess
import sys

EXE = pathlib.Path(
    str(pathlib.Path(__file__).resolve().parent.parent / "target" / "release" / "devzymer.exe")
)
BASE = pathlib.Path(tempfile.gettempdir()) / "dz-journeys"


def fresh(name):
    root = BASE / name
    if root.exists():
        shutil.rmtree(root, ignore_errors=True)
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "package.json").write_text('{"name":"demo"}', encoding="utf-8")
    (root / "src" / "index.ts").write_text(
        "export function ola(nome: string) { return nome; }\n", encoding="utf-8"
    )
    return root


def run(root, *args, expect_ok=True):
    proc = subprocess.run(
        [str(EXE), *args, "--root", str(root)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if expect_ok and proc.returncode != 0:
        print(f"    FALHOU: {' '.join(args)}")
        print("    stdout:", proc.stdout[:500])
        print("    stderr:", proc.stderr[:500])
    return proc


def check(label, condition, detail=""):
    print(f"  [{'ok' if condition else 'FALHA'}] {label} {detail}")
    return condition


results = []

# 1. Claude Code apenas
root = fresh("claude-only")
proc = run(root, "init", "--agents", "claude")
ok = proc.returncode == 0
ok &= check(".mcp.json criado", (root / ".mcp.json").exists())
ok &= check("CLAUDE.md criado", (root / "CLAUDE.md").exists())
ok &= check("sem config do codex", not (root / ".codex").exists())
ok &= check("sem opencode.json", not (root / "opencode.json").exists())
results.append(("claude apenas", ok))

# 2. Codex apenas
root = fresh("codex-only")
proc = run(root, "init", "--agents", "codex")
ok = proc.returncode == 0
ok &= check("config do codex criada", (root / ".codex" / "config.toml").exists())
ok &= check("AGENTS.md criado", (root / "AGENTS.md").exists())
ok &= check("sem .mcp.json", not (root / ".mcp.json").exists())
results.append(("codex apenas", ok))

# 3. OpenCode apenas
root = fresh("opencode-only")
proc = run(root, "init", "--agents", "opencode")
ok = proc.returncode == 0
ok &= check("opencode.json criado", (root / "opencode.json").exists())
ok &= check("sem .mcp.json", not (root / ".mcp.json").exists())
results.append(("opencode apenas", ok))

# 4. Claude + Codex
root = fresh("claude-codex")
proc = run(root, "init", "--agents", "claude,codex")
ok = proc.returncode == 0
ok &= check(".mcp.json e codex", (root / ".mcp.json").exists() and (root / ".codex" / "config.toml").exists())
results.append(("claude + codex", ok))

# 5. Todos os tres
root = fresh("all-three")
proc = run(root, "init", "--agents", "claude,codex,opencode")
ok = proc.returncode == 0
ok &= check(
    "tres configuracoes",
    (root / ".mcp.json").exists()
    and (root / ".codex" / "config.toml").exists()
    and (root / "opencode.json").exists(),
)
indexes = [p for p in root.rglob(".devzymer") if p.is_dir()]
ok &= check("um unico indice de projeto", len(indexes) == 1, f"({len(indexes)})")
results.append(("os tres agentes", ok))

# 6. Reinstallacao idempotente
proc = run(root, "init", "--agents", "claude,codex,opencode")
ok = proc.returncode == 0
ok &= check("reinstallacao sem duplicar", (root / "opencode.json").read_text(encoding="utf-8").count('"devzymer"') == 1)
ok &= check(
    "reinstallacao sem duplicar codex",
    (root / ".codex" / "config.toml").read_text(encoding="utf-8").count("[mcp_servers.devzymer]") == 1,
)
results.append(("reinstallacao", ok))

# 7. integrations list / remove
proc = run(root, "integrations", "list")
ok = proc.returncode == 0 and "codex" in proc.stdout
proc = run(root, "integrations", "remove", "codex")
ok &= check("codex removido", "[mcp_servers.devzymer]" not in (root / ".codex" / "config.toml").read_text(encoding="utf-8"))
ok &= check("claude preservado", (root / ".mcp.json").exists())
results.append(("integrations list/remove", ok))

# 8. integrations add
proc = run(root, "integrations", "add", "codex")
ok &= check("codex reinstallado", "[mcp_servers.devzymer]" in (root / ".codex" / "config.toml").read_text(encoding="utf-8"))
results.append(("integrations add", ok))

# 9. Sem terminal interativo
root = fresh("non-interactive")
proc = run(root, "init")
ok = proc.returncode == 0
ok &= check("aviso de fallback", "sem terminal interativo" in proc.stdout)
ok &= check("claude instalado por compatibilidade", (root / ".mcp.json").exists())
results.append(("sem terminal", ok))

# 10. Agente inexistente
root = fresh("bad-agent")
proc = run(root, "init", "--agents", "emacs", expect_ok=False)
ok = proc.returncode != 0 and "agente desconhecido" in (proc.stderr + proc.stdout)
results.append(("agente invalido", ok))

# 11. Configuracao existente preservada
root = fresh("preserve")
(root / "opencode.json").write_text(
    json.dumps(
        {
            "$schema": "https://opencode.ai/config.json",
            "mcp": {"servers": {"outro": {"type": "local", "command": ["x"]}}},
            "model": "meu/modelo",
        },
        indent=2,
    ),
    encoding="utf-8",
)
proc = run(root, "init", "--agents", "opencode")
content = (root / "opencode.json").read_text(encoding="utf-8")
ok = proc.returncode == 0
ok &= check("servidor existente preservado", '"outro"' in content)
ok &= check("modelo preservado", '"meu/modelo"' in content)
ok &= check("devzymer adicionado", '"devzymer"' in content)
results.append(("configuracao preservada", ok))

# 12. JSONC com comentario preservado
root = fresh("jsonc")
(root / "opencode.jsonc").write_text(
    '{\n  // configuracao do usuario\n  "mcp": {\n    "servers": {\n      "outro": {"type": "local"}\n    }\n  }\n}\n',
    encoding="utf-8",
)
proc = run(root, "init", "--agents", "opencode")
content = (root / "opencode.jsonc").read_text(encoding="utf-8")
ok = proc.returncode == 0
ok &= check("comentario preservado", "// configuracao do usuario" in content)
ok &= check("servidor anterior preservado", '"outro"' in content)
ok &= check("devzymer adicionado no jsonc", '"devzymer"' in content)
results.append(("jsonc preservado", ok))

# 13. doctor com os agentes
root = fresh("doctor")
run(root, "init", "--agents", "claude,codex,opencode")
proc = run(root, "doctor")
ok = proc.returncode == 0
ok &= check("doctor lista os agentes", "agente:codex" in proc.stdout and "agente:opencode" in proc.stdout)
results.append(("doctor", ok))

# 14. Troca de repositorio
root_a = fresh("repo-a")
root_b = fresh("repo-b")
run(root_a, "init", "--agents", "claude")
run(root_b, "init", "--agents", "claude")
proc = subprocess.run(
    [str(EXE), "resolve", "--root", str(root_b), "ola"],
    capture_output=True,
    text=True,
    encoding="utf-8",
    errors="replace",
)
ok = proc.returncode == 0 and "ola" in proc.stdout
ok &= check("indice do repositorio correto", "ola" in proc.stdout)
results.append(("troca de repositorio", ok))

print()
print("resumo:")
falhas = [name for name, ok in results if not ok]
for name, ok in results:
    print(f"  {'ok ' if ok else 'FALHA'} {name}")
sys.exit(1 if falhas else 0)
