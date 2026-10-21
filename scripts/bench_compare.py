"""Compares two benchmark reports and prints the deltas that matter."""

from __future__ import annotations

import json
import sys
from pathlib import Path


def load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    before = load(Path(sys.argv[1]))
    after = load(Path(sys.argv[2]))

    def show(label: str, old, new, fmt="{}"):
        print(f"{label:<34} {fmt.format(old):>22} -> {fmt.format(new):>22}")

    show("tarefas unicas", before.get("tasks_run", "-"), after.get("unique_tasks", "-"))
    show("execucoes", before.get("tasks_run", "-"), after.get("executions", "-"))
    show("acertos", before.get("hits", "-"), after.get("hits", "-"))
    show(
        "hit rate %",
        round(before.get("hit_rate", 0) * 100, 2),
        round(after.get("hit_rate", 0) * 100, 2),
    )
    show("mrr", round(before.get("mrr", 0), 4), round(after.get("mrr", 0), 4))
    show(
        "latencia media ms",
        round(before.get("mean_latency_ms", 0), 2),
        round(after.get("latency", {}).get("mean_ms", 0), 2),
    )
    show(
        "reducao tokens %",
        round(before.get("token_reduction_pct", 0), 2),
        after.get("tokens", {}).get("reduction_pct"),
    )
    show(
        "tarefas com reducao negativa",
        "-",
        after.get("tokens", {}).get("tasks_with_negative_reduction", "-"),
    )
    show(
        "tarefas sem linha de base",
        "-",
        after.get("tokens", {}).get("tasks_without_baseline", "-"),
    )
    show("cache acertos", "-", after.get("cache", {}).get("hits", "-"))
    show("cache falhas", "-", after.get("cache", {}).get("misses", "-"))

    print("\npor cenario (depois):")
    for kind in after.get("by_kind", []):
        reduction = kind.get("reduction_pct")
        reduction_text = "n/d" if reduction is None else f"{reduction:+.2f}%"
        print(
            f"  {kind['kind']:<24} tarefas={kind['tasks']:<4} verificadas={kind['verified_tasks']:<4} "
            f"acertos={kind['hits']:<4} recall@1={kind['recall_at_1']*100:5.1f}% "
            f"recall@5={kind['recall_at_5']*100:5.1f}% reducao={reduction_text}"
        )

    print("\nfalhas (depois):")
    for result in after.get("results", []):
        if result.get("failure_reason"):
            print(
                f"  [{result['task']['kind']}] {result['task']['query']!r} "
                f"-> {result['failure_reason']}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
