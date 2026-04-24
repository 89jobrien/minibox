#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
gen-class-diagrams.py — Generate Mermaid classDiagram blocks from workspace Rust sources.

Parses crates/*/src/**/*.rs using regex (no syn dependency).
Extracts pub structs, pub enums, and pub traits with their members.
Emits one Mermaid classDiagram block per crate, appended to docs/diagrams.html
as diagram-card sections matching the existing card style.

Re-running is idempotent: replaces content from <!-- generated --> to </body>.
"""

import re
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent
CRATES_DIR = REPO_ROOT / "crates"
OUTPUT_HTML = REPO_ROOT / "docs" / "diagrams.html"

# Maximum characters for a trait method signature line in the diagram.
METHOD_TRUNCATE = 60

# ---------------------------------------------------------------------------
# Regex patterns
# ---------------------------------------------------------------------------

# Match `pub struct Foo` or `pub(crate) struct Foo` — capture name.
RE_STRUCT = re.compile(
    r"^\s*pub(?:\([^)]+\))?\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)

# Match `pub enum Foo`.
RE_ENUM = re.compile(
    r"^\s*pub(?:\([^)]+\))?\s+enum\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)

# Match `pub trait Foo`.
RE_TRAIT = re.compile(
    r"^\s*pub(?:\([^)]+\))?\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)",
    re.MULTILINE,
)

# Match a pub field inside a struct body: `    pub field_name: SomeType,`
RE_PUB_FIELD = re.compile(
    r"^\s{4,}pub\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([^,\n]+)",
    re.MULTILINE,
)

# Match an enum variant (indented, starts with uppercase or underscore, no `pub`).
RE_VARIANT = re.compile(
    r"^\s{4}([A-Z_][A-Za-z0-9_]*)(?:\s*[({,]|$)",
    re.MULTILINE,
)

# Match a fn signature inside a trait body.
RE_FN = re.compile(
    r"^\s{4}(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*[^;{]*)",
    re.MULTILINE,
)

# ---------------------------------------------------------------------------
# Extraction helpers
# ---------------------------------------------------------------------------


def extract_block(source: str, open_pos: int) -> str:
    """Return the content between the first { after open_pos and its matching }."""
    start = source.find("{", open_pos)
    if start == -1:
        return ""
    depth = 0
    for i, ch in enumerate(source[start:], start=start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return source[start + 1 : i]
    return source[start + 1 :]


def parse_structs(source: str) -> list[dict]:
    results = []
    for m in RE_STRUCT.finditer(source):
        name = m.group(1)
        body = extract_block(source, m.end())
        fields = []
        for fm in RE_PUB_FIELD.finditer(body):
            field_name = fm.group(1)
            field_type = fm.group(2).strip().rstrip(",")
            fields.append((field_name, field_type))
        results.append({"kind": "struct", "name": name, "fields": fields})
    return results


def parse_enums(source: str) -> list[dict]:
    results = []
    for m in RE_ENUM.finditer(source):
        name = m.group(1)
        body = extract_block(source, m.end())
        variants = [vm.group(1) for vm in RE_VARIANT.finditer(body)]
        results.append({"kind": "enum", "name": name, "variants": variants})
    return results


def parse_traits(source: str) -> list[dict]:
    results = []
    for m in RE_TRAIT.finditer(source):
        name = m.group(1)
        body = extract_block(source, m.end())
        methods = []
        for fm in RE_FN.finditer(body):
            sig = fm.group(1).strip()
            if len(sig) > METHOD_TRUNCATE:
                sig = sig[: METHOD_TRUNCATE - 3] + "..."
            methods.append(sig)
        results.append({"kind": "trait", "name": name, "methods": methods})
    return results


def parse_file(path: Path) -> dict:
    """Parse a single .rs file; return dict with structs/enums/traits lists."""
    try:
        source = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return {"structs": [], "enums": [], "traits": []}
    return {
        "structs": parse_structs(source),
        "enums": parse_enums(source),
        "traits": parse_traits(source),
    }


def merge(acc: dict, parsed: dict) -> None:
    """Merge parsed file results into accumulator, deduplicating by name."""
    for kind in ("structs", "enums", "traits"):
        seen = {item["name"] for item in acc[kind]}
        for item in parsed[kind]:
            if item["name"] not in seen:
                acc[kind].append(item)
                seen.add(item["name"])


# ---------------------------------------------------------------------------
# Mermaid rendering
# ---------------------------------------------------------------------------


def mermaid_safe(name: str) -> str:
    """Escape backticks and quotes that break Mermaid class names."""
    return name.replace("`", "").replace('"', "")


def render_mermaid(crate_data: dict) -> str:
    """Render a Mermaid classDiagram block for one crate."""
    lines = ["classDiagram"]

    for item in crate_data["structs"]:
        name = mermaid_safe(item["name"])
        lines.append(f"  class {name} {{")
        lines.append("    <<struct>>")
        for field_name, field_type in item["fields"][:20]:
            ft = mermaid_safe(field_type)
            lines.append(f"    +{field_name} {ft}")
        lines.append("  }")

    for item in crate_data["enums"]:
        name = mermaid_safe(item["name"])
        lines.append(f"  class {name} {{")
        lines.append("    <<enum>>")
        for variant in item["variants"][:20]:
            lines.append(f"    +{mermaid_safe(variant)}")
        lines.append("  }")

    for item in crate_data["traits"]:
        name = mermaid_safe(item["name"])
        lines.append(f"  class {name} {{")
        lines.append("    <<trait>>")
        for method in item["methods"][:20]:
            sig = mermaid_safe(method)
            lines.append(f"    +{sig}")
        lines.append("  }")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# HTML card rendering
# ---------------------------------------------------------------------------

CARD_TEMPLATE = """\
  <div class="diagram-card" id="class-{crate_id}">
    <div class="diagram-card-header">
      <div>
        <div style="display:flex;align-items:center;gap:0.6rem">
          <span class="diagram-number">{number}</span>
          <span class="diagram-title">{crate_name} — Class Diagram</span>
        </div>
        <div class="diagram-desc">
          Auto-generated from <code>crates/{crate_name}/src/**/*.rs</code> —
          pub structs ({n_structs}), pub enums ({n_enums}), pub traits ({n_traits}).
        </div>
        <div class="badge-row">
          <span class="badge badge-purple">classDiagram</span>
          <span class="badge badge-green">{n_structs} structs</span>
          <span class="badge badge-yellow">{n_traits} traits</span>
        </div>
      </div>
    </div>
    <div class="diagram-body">
      <div class="mermaid">
{mermaid}
      </div>
    </div>
  </div>"""


def render_card(crate_name: str, index: int, crate_data: dict) -> str:
    crate_id = crate_name.replace("-", "_")
    number = str(index).zfill(2)
    mermaid = render_mermaid(crate_data)
    # Indent mermaid block to sit inside the <div class="mermaid"> tag.
    indented = "\n".join(f"        {line}" if line.strip() else "" for line in mermaid.splitlines())
    return CARD_TEMPLATE.format(
        crate_id=crate_id,
        number=number,
        crate_name=crate_name,
        n_structs=len(crate_data["structs"]),
        n_enums=len(crate_data["enums"]),
        n_traits=len(crate_data["traits"]),
        mermaid=indented,
    )


# ---------------------------------------------------------------------------
# HTML base template (used only when docs/diagrams.html does not exist)
# ---------------------------------------------------------------------------

BASE_HTML = """\
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Minibox — Class Diagrams</title>
  <script src="https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js"></script>
  <style>
    :root {
      --bg: #0f1117; --surface: #1a1d27; --border: #2a2d3e;
      --accent: #7c6af7; --accent2: #3ecf8e; --text: #e2e4ef;
      --muted: #8b8fa8; --danger: #f87171; --warn: #fbbf24;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: 'Inter', system-ui, sans-serif; background: var(--bg);
           color: var(--text); min-height: 100vh; }
    header { background: var(--surface); border-bottom: 1px solid var(--border);
             padding: 1.5rem 2rem; }
    header .logo { font-size: 1.4rem; font-weight: 700; color: var(--accent); }
    header .subtitle { color: var(--muted); font-size: 0.875rem; }
    main { max-width: 1200px; margin: 0 auto; padding: 2rem;
           display: flex; flex-direction: column; gap: 3rem; }
    .diagram-card { background: var(--surface); border: 1px solid var(--border);
                    border-radius: 12px; overflow: hidden; }
    .diagram-card-header { padding: 1.25rem 1.5rem; border-bottom: 1px solid var(--border); }
    .diagram-number { background: var(--accent); color: #fff; font-size: 0.7rem;
                      font-weight: 700; padding: 0.2rem 0.5rem; border-radius: 4px; }
    .diagram-title { font-size: 1.1rem; font-weight: 600; color: var(--text); }
    .diagram-desc { color: var(--muted); font-size: 0.85rem; margin-top: 0.35rem;
                    line-height: 1.6; }
    .diagram-body { padding: 1.5rem; background: #fff; border-radius: 0 0 12px 12px; }
    .diagram-body .mermaid { display: flex; justify-content: center; }
    .badge-row { display: flex; gap: 0.5rem; margin-top: 0.5rem; flex-wrap: wrap; }
    .badge { font-size: 0.7rem; padding: 0.15rem 0.5rem; border-radius: 4px;
             font-weight: 600; }
    .badge-green  { background: rgba(62,207,142,0.15); color: var(--accent2);
                    border: 1px solid rgba(62,207,142,0.3); }
    .badge-purple { background: rgba(124,106,247,0.15); color: var(--accent);
                    border: 1px solid rgba(124,106,247,0.3); }
    .badge-yellow { background: rgba(251,191,36,0.15);  color: var(--warn);
                    border: 1px solid rgba(251,191,36,0.3); }
    footer { text-align: center; color: var(--muted); font-size: 0.78rem;
             padding: 2rem; border-top: 1px solid var(--border); margin-top: 2rem; }
  </style>
</head>
<body>
<header>
  <div class="logo">minibox</div>
  <div class="subtitle">Class Diagrams — Auto-generated</div>
</header>
<main>
</main>
<footer>Generated by scripts/gen-class-diagrams.py</footer>
</body>
</html>
"""

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def collect_crate_data(crate_dir: Path) -> dict:
    """Walk src/**/*.rs in a crate directory and merge all parsed results."""
    acc: dict = {"structs": [], "enums": [], "traits": []}
    src_dir = crate_dir / "src"
    if not src_dir.is_dir():
        return acc
    for rs_file in sorted(src_dir.rglob("*.rs")):
        parsed = parse_file(rs_file)
        merge(acc, parsed)
    return acc


def main() -> int:
    crate_dirs = sorted(
        d for d in CRATES_DIR.iterdir() if d.is_dir() and (d / "src").is_dir()
    )

    if not crate_dirs:
        print(f"No crates found under {CRATES_DIR}", file=sys.stderr)
        return 1

    print(f"Found {len(crate_dirs)} crates.")

    # Build per-crate data and cards.
    cards: list[str] = []
    for idx, crate_dir in enumerate(crate_dirs, start=1):
        crate_name = crate_dir.name
        data = collect_crate_data(crate_dir)
        total = len(data["structs"]) + len(data["enums"]) + len(data["traits"])
        if total == 0:
            print(f"  [{idx:02d}] {crate_name}: no public types found — skipping")
            continue
        print(
            f"  [{idx:02d}] {crate_name}: "
            f"{len(data['structs'])} structs, "
            f"{len(data['enums'])} enums, "
            f"{len(data['traits'])} traits"
        )
        cards.append(render_card(crate_name, idx, data))

    if not cards:
        print("No cards generated.", file=sys.stderr)
        return 1

    # Read existing HTML or use base template.
    if OUTPUT_HTML.exists():
        html = OUTPUT_HTML.read_text(encoding="utf-8")
    else:
        OUTPUT_HTML.parent.mkdir(parents=True, exist_ok=True)
        html = BASE_HTML

    generated_block = (
        "\n<!-- generated -->\n"
        + "\n\n".join(cards)
        + "\n<!-- /generated -->\n"
    )

    if "<!-- generated -->" in html:
        # Replace existing generated block (idempotent re-run).
        html = re.sub(
            r"<!-- generated -->.*?<!-- /generated -->",
            generated_block.strip(),
            html,
            flags=re.DOTALL,
        )
    else:
        # Insert before </body>.
        if "</body>" in html:
            html = html.replace("</body>", generated_block + "</body>")
        else:
            html += generated_block

    OUTPUT_HTML.write_text(html, encoding="utf-8")
    print(f"Written: {OUTPUT_HTML}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
