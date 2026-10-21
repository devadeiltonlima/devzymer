import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "docs" / "demo.svg"

W = 960
H = 392
PAD = 26
CW = 6.9
LH = 18
TOP = 70
FONT = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace"

BG = "#0d1117"
BAR = "#161b22"
EDGE = "#30363d"
TEXT = "#e6edf3"
MUTED = "#8b949e"
GREEN = "#3fb950"
BLUE = "#79c0ff"
AMBER = "#d29922"
RED = "#f85149"

TYPE_STEP = 0.028
PRE_TYPE = 0.4
POST_TYPE = 0.4
LINE_STEP = 0.24
HOLD = 2.4
GAP = 0.4


def seg(*parts):
    return [(text, color) for text, color in parts]


def resolve_line(path, symbol, kind, origin):
    origin_color = GREEN if origin == "confirmed" else MUTED
    return seg(
        (path, BLUE),
        (" | ", MUTED),
        (symbol, TEXT),
        (" | ", MUTED),
        (kind, MUTED),
        (" | ", MUTED),
        (origin, origin_color),
    )


def kv(key, value):
    return seg((key + ": ", MUTED), (value, TEXT))


def code(number, source):
    return seg((f"{number:>6} | ".replace(" ", "\u00a0"), MUTED), (source.replace(" ", "\u00a0"), TEXT))


def cmd(text):
    return {"kind": "cmd", "text": text}


def out(lines):
    return {"kind": "out", "lines": lines}


def bars(rows):
    return {"kind": "bars", "rows": rows}


SCENES = [
    {
        "label": "1  localizar",
        "steps": [
            cmd('devzymer resolve "Store fts search lexical" --root . --limit 4'),
            out(
                [
                    seg(
                        ("arquivo: ", MUTED),
                        ("crates/devzymer-core/src/store.rs", BLUE),
                        (" | 110 símbolo(s) | nome do arquivo na consulta: Store", MUTED),
                    ),
                    resolve_line(
                        "crates/devzymer-core/src/store.rs:1472-1496",
                        "tests.replacing_symbols_keeps_the_lexical_index_consistent",
                        "function",
                        "heuristic",
                    ),
                    resolve_line(
                        "crates/devzymer-core/src/store.rs:1445-1469",
                        "tests.the_lexical_index_keeps_the_words_of_a_camel_case_name",
                        "function",
                        "heuristic",
                    ),
                    resolve_line(
                        "crates/devzymer-core/src/store.rs:748-802",
                        "Store.fts_search_terms",
                        "function",
                        "heuristic",
                    ),
                    resolve_line(
                        "crates/devzymer-core/src/store.rs:341-361",
                        "Store.rebuild_lexical_index",
                        "function",
                        "heuristic",
                    ),
                ]
            ),
        ],
    },
    {
        "label": "2  o que o agente recebe (MCP)",
        "steps": [
            cmd('context_resolve  query="Store.fts_search_terms"'),
            out(
                [
                    seg(
                        ("arquivo: ", MUTED),
                        ("crates/devzymer-core/src/store.rs", BLUE),
                        (" | 110 símbolo(s) | nome do arquivo", MUTED),
                    ),
                    resolve_line(
                        "crates/devzymer-core/src/store.rs:748-802",
                        "Store.fts_search_terms",
                        "function",
                        "confirmed",
                    ),
                ]
            ),
            cmd('context_read  symbol="Store.fts_search_terms"'),
            out(
                [
                    seg(
                        ("crates/devzymer-core/src/store.rs:748-787", BLUE),
                        (" sha=db956bdb7ab75e0b bytes=1363 tokens~431", MUTED),
                    ),
                    code(748, "    pub fn fts_search_terms("),
                    code(749, "        &self,"),
                    code(750, "        terms: &[String],"),
                    code(751, "        limit: usize,"),
                    code(752, "        weights: [f64; 6],"),
                    seg(("\u00a0" * 7 + "...\u00a0\u00a0só o trecho pedido, não o arquivo inteiro", AMBER)),
                ]
            ),
        ],
    },
    {
        "label": "3  índice saudável",
        "steps": [
            cmd("devzymer status --root ."),
            out(
                [
                    kv("índice", "406 arquivos, 1447 símbolos, 6718 relações, revisão 210"),
                    kv("idiomas", "unknown=318, rust=68, python=20"),
                    kv("atualidade", "0 arquivo(s) alterado(s) desde o último índice"),
                    kv("integração MCP", "instalada | Claude Code CLI: presente"),
                    kv("permissão das ferramentas MCP", "concedida"),
                ]
            ),
            cmd("devzymer doctor --root . | tail -1"),
            out([seg(("12 verificação(ões), 0 falha(s)", GREEN))]),
        ],
    },
    {
        "label": "4  medir",
        "steps": [
            cmd("devzymer benchmark --root . --tasks 16 --seed 20260319"),
            out(
                [
                    kv("tarefas únicas", "113 | execuções: 113 | verificadas: 92"),
                    kv("acerto no top 5", "90 (97.8%) | recall@1 88.0% | recall@5 92.4% | MRR 0.897"),
                    kv("latência", "média 3.4 ms | p50 2.0 | p95 10.0 | p99 15.0"),
                    kv("tokens", "91009 de contexto | 429632 de linha de base | redução +78.82%"),
                ]
            ),
            bars(
                [
                    ("sem índice", 429632, RED),
                    ("com devzymer", 91009, GREEN),
                ]
            ),
        ],
    },
]


def esc(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def fmt(value):
    return f"{value:.5f}".rstrip("0").rstrip(".")


def plan():
    t = 0.0
    scenes = []
    for scene in SCENES:
        start = t
        t += 0.35
        y = TOP
        items = []
        for step in scene["steps"]:
            if step["kind"] == "cmd":
                t += PRE_TYPE
                chars = len(step["text"])
                items.append(
                    {"kind": "cmd", "text": step["text"], "y": y, "t0": t, "t1": t + chars * TYPE_STEP}
                )
                t += chars * TYPE_STEP + POST_TYPE
                y += LH
            elif step["kind"] == "out":
                for line in step["lines"]:
                    items.append({"kind": "line", "segments": line, "y": y, "t0": t})
                    t += LINE_STEP
                    y += LH
                y += 4
            else:
                y += 4
                for label, value, color in step["rows"]:
                    items.append(
                        {
                            "kind": "bar",
                            "label": label,
                            "value": value,
                            "color": color,
                            "y": y,
                            "t0": t,
                            "max": max(row[1] for row in step["rows"]),
                        }
                    )
                    t += 0.9
                    y += LH + 6
        t += HOLD
        scenes.append({"label": scene["label"], "start": start, "end": t, "items": items})
        t += GAP
    return scenes, t


def anim_opacity(times, total):
    keys = ";".join(fmt(x / total) for x in [0.0] + times)
    values = ";".join(["0"] + ["1" if i % 2 == 0 else "0" for i in range(len(times))])
    return (
        f'<animate attributeName="opacity" calcMode="discrete" values="{values}" '
        f'keyTimes="{keys}" dur="{fmt(total)}s" repeatCount="indefinite"/>'
    )


def render():
    scenes, total = plan()
    parts = []
    defs = []
    body = []
    clip_id = 0

    for index, scene in enumerate(scenes):
        group = [f'<g opacity="0">', "  " + anim_opacity([scene["start"], scene["end"]], total)]
        for item in scene["items"]:
            if item["kind"] == "cmd":
                clip_id += 1
                chars = len(item["text"])
                y = item["y"]
                times = [0.0] + [item["t0"] + k * TYPE_STEP for k in range(chars)] + [scene["end"]]
                widths = [0.0] + [(k + 1) * CW for k in range(chars)] + [0.0]
                keys = ";".join(fmt(x / total) for x in times)
                widths_attr = ";".join(fmt(w) for w in widths)
                cursor_x = ";".join(fmt(PAD + 2 * CW + w) for w in widths)
                defs.append(
                    f'<clipPath id="k{clip_id}"><rect x="{PAD + 2 * CW}" y="{y - 13}" width="0" height="{LH}">'
                    f'<animate attributeName="width" calcMode="discrete" values="{widths_attr}" keyTimes="{keys}" '
                    f'dur="{fmt(total)}s" repeatCount="indefinite"/></rect></clipPath>'
                )
                group.append(
                    f'  <text x="{PAD}" y="{y}" fill="{GREEN}" font-size="12" textLength="{fmt(CW)}">$</text>'
                )
                group.append(
                    f'  <text x="{PAD + 2 * CW}" y="{y}" fill="{TEXT}" font-size="12" '
                    f'textLength="{fmt(chars * CW)}" lengthAdjust="spacing" clip-path="url(#k{clip_id})">{esc(item["text"])}</text>'
                )
                group.append(
                    f'  <rect x="{PAD + 2 * CW}" y="{y - 11}" width="{CW}" height="14" fill="{TEXT}" opacity="0.85">'
                    f'<animate attributeName="x" calcMode="discrete" values="{cursor_x}" keyTimes="{keys}" '
                    f'dur="{fmt(total)}s" repeatCount="indefinite"/></rect>'
                )
            elif item["kind"] == "line":
                text = "".join(part[0] for part in item["segments"])
                spans = "".join(
                    f'<tspan fill="{color}">{esc(chunk)}</tspan>' for chunk, color in item["segments"]
                )
                group.append(
                    f'  <text x="{PAD}" y="{item["y"]}" font-size="12" opacity="0" '
                    f'textLength="{fmt(len(text) * CW)}" lengthAdjust="spacing">{spans}'
                    f'<animate attributeName="opacity" calcMode="discrete" values="0;1" '
                    f'keyTimes="0;{fmt(item["t0"] / total)}" dur="{fmt(total)}s" repeatCount="indefinite"/></text>'
                )
            else:
                y = item["y"]
                full = 520
                width = max(6.0, full * item["value"] / item["max"])
                t0 = item["t0"]
                keys = ";".join(fmt(x / total) for x in [0.0, t0, t0 + 0.8, total])
                values = ";".join(fmt(x) for x in [0.0, 0.0, width, width])
                group.append(
                    f'  <text x="{PAD}" y="{y + 11}" fill="{MUTED}" font-size="12" textLength="{fmt(13 * CW)}" '
                    f'lengthAdjust="spacing" opacity="0">{esc(item["label"])}'
                    f'<animate attributeName="opacity" calcMode="discrete" values="0;1" '
                    f'keyTimes="0;{fmt(t0 / total)}" dur="{fmt(total)}s" repeatCount="indefinite"/></text>'
                )
                group.append(
                    f'  <rect x="{fmt(PAD + 14 * CW)}" y="{y}" width="0" height="14" rx="4" fill="{item["color"]}">'
                    f'<animate attributeName="width" calcMode="linear" values="{values}" keyTimes="{keys}" '
                    f'dur="{fmt(total)}s" repeatCount="indefinite"/></rect>'
                )
                group.append(
                    f'  <text x="{fmt(PAD + 14 * CW + width + 10)}" y="{y + 11}" fill="{TEXT}" font-size="12" opacity="0">'
                    f'{item["value"]:,}'.replace(",", ".")
                    + f' tokens<animate attributeName="opacity" calcMode="discrete" values="0;1" '
                    f'keyTimes="0;{fmt((t0 + 0.8) / total)}" dur="{fmt(total)}s" repeatCount="indefinite"/></text>'
                )
        group.append("</g>")
        body.append("\n".join(group))

        label = scene["label"]
        body.append(
            f'<text x="{W - PAD}" y="23" text-anchor="end" fill="{AMBER}" font-size="12" opacity="0">{esc(label)}'
            f'{anim_opacity([scene["start"], scene["end"]], total)}</text>'
        )

    dots = []
    for index, scene in enumerate(scenes):
        cx = W / 2 - 36 + index * 24
        dots.append(
            f'<circle cx="{fmt(cx)}" cy="{H - 20}" r="4" fill="{EDGE}"/>'
            f'<circle cx="{fmt(cx)}" cy="{H - 20}" r="4" fill="{GREEN}" opacity="0">'
            f'{anim_opacity([scene["start"], scene["end"]], total)}</circle>'
        )

    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" '
        f'font-family="{FONT}" xml:space="preserve" role="img" aria-label="Demonstração animada do devzymer: localizar, o que o agente recebe, estado do índice e benchmark">',
        "<title>devzymer em ação (saídas reais do próprio repositório)</title>",
        "<defs>",
        *defs,
        "</defs>",
        f'<rect width="{W}" height="{H}" rx="12" fill="{BG}" stroke="{EDGE}"/>',
        f'<path d="M0 12a12 12 0 0 1 12-12h{W - 24}a12 12 0 0 1 12 12v22H0z" fill="{BAR}"/>',
        f'<line x1="0" y1="34" x2="{W}" y2="34" stroke="{EDGE}"/>',
        '<circle cx="22" cy="17" r="5.5" fill="#ff5f56"/>',
        '<circle cx="42" cy="17" r="5.5" fill="#ffbd2e"/>',
        '<circle cx="62" cy="17" r="5.5" fill="#27c93f"/>',
        f'<text x="{W / 2}" y="21" text-anchor="middle" fill="{MUTED}" font-size="12">devzymer · repositório devzymer</text>',
        *body,
        *dots,
        f'<text x="{PAD}" y="{H - 16}" fill="{MUTED}" font-size="11">saídas reais, sem edição de conteúdo</text>',
        f'<text x="{W - PAD}" y="{H - 16}" text-anchor="end" fill="{MUTED}" font-size="11">tokens: estimativa local</text>',
        "</svg>",
    ]
    return "\n".join(svg), total


if __name__ == "__main__":
    content, total = render()
    OUT.write_text(content, encoding="utf-8")
    print(f"{OUT} | {len(content)} bytes | loop de {total:.1f}s")
