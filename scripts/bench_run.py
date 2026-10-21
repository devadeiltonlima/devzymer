"""Runs the benchmark on a real repository and prints the diagnostic summary.

Usage: python scripts/bench_run.py <project> [tasks] [seed]
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


def main() -> int:
    project = Path(sys.argv[1])
    tasks = sys.argv[2] if len(sys.argv) > 2 else "20"
    seed = sys.argv[3] if len(sys.argv) > 3 else "20260319"
    output = project / ".devzymer" / "bench_after.json"

    subprocess.run(
        [
            str(_binary()),
            "benchmark",
            "--root",
            str(project),
            "--tasks",
            tasks,
            "--seed",
            seed,
            "--format",
            "json",
            "--output",
            str(output),
        ],
        check=True,
    )

    report = json.loads(output.read_text(encoding="utf-8"))
    print(f"repositorio: {report['repository']['name']}")
    print(
        f"indice: {report['repository']['files']} arquivos, "
        f"{report['repository']['symbols']} simbolos, revisao {report['repository']['index_revision']}"
    )
    print(f"semente: {report['seed']} | estimador: {report['token_estimator']}")
    print(
        f"tarefas unicas: {report['unique_tasks']} | execucoes: {report['executions']} | "
        f"verificadas: {report['verified_tasks']} | nao verificadas: {report['unverified_tasks']}"
    )
    print(
        f"acerto no top {report['top_k']}: {report['hits']} ({report['hit_rate']*100:.1f}%) | "
        f"recall@1 {report['recall_at_1']*100:.1f}% | recall@5 {report['recall_at_5']*100:.1f}% | "
        f"MRR {report['mrr']:.3f}"
    )
    latency = report["latency"]
    print(
        f"latencia: media {latency['mean_ms']:.2f} ms | p50 {latency['p50_ms']:.1f} | "
        f"p95 {latency['p95_ms']:.1f} | p99 {latency['p99_ms']:.1f}"
    )
    tokens = report["tokens"]
    reduction = tokens["reduction_pct"]
    print(
        f"tokens: {tokens['context_tokens']} de contexto | {tokens['baseline_tokens']} de linha de base | "
        f"reducao {'n/d' if reduction is None else f'{reduction:+.2f}%'} | "
        f"negativas: {tokens['tasks_with_negative_reduction']} | "
        f"sem linha de base: {tokens['tasks_without_baseline']}"
    )
    cache = report["cache"]
    print(
        f"cache: {cache['hits']} acertos, {cache['misses']} falhas | "
        f"latencia acerto {cache['mean_latency_hit_ms']:.2f} ms | "
        f"falha {cache['mean_latency_miss_ms']:.2f} ms | "
        f"economia de tokens do cache: {cache['token_saving_from_cache_pct']:+.2f}%"
    )

    print("\npor cenario:")
    for kind in report["by_kind"]:
        value = kind["reduction_pct"]
        print(
            f"  {kind['kind']:<22} tarefas={kind['tasks']:<3} verificadas={kind['verified_tasks']:<3} "
            f"acertos={kind['hits']:<3} recall@1={kind['recall_at_1']*100:5.1f}% "
            f"recall@5={kind['recall_at_5']*100:5.1f}% "
            f"reducao={'n/d' if value is None else f'{value:+.2f}%'}"
        )

    failures = [r for r in report["results"] if r.get("failure_reason")]
    if failures:
        print("\nfalhas:")
        for failure in failures:
            print(
                f"  [{failure['task']['kind']}] {failure['task']['query']!r} "
                f"-> {failure['failure_reason']}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
