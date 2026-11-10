#!/usr/bin/env python3
"""A Rust file over 3000 lines may not grow compared with the branch's base (docs/layers.md#file-size).

Big files slow incremental builds (game.rs, gfx.rs). Any file may grow up to the limit; a file over it may shrink or
stay the same. A renamed file is compared with its old name, and a new file counts from zero. Files under
third_party/ and files that say they are generated (`@generated` in their first lines) are not checked. There is no
general exemption list. The sync-only ratchet in tools/file_size_baseline.txt records
restored upstream growth at its exact size, and only a sync PR uses it: a changed sync-baseline
header declares the upstream SHA, permits new or raised entries, makes every entry a cap and makes an
entry above the current size fail. Any other PR ignores the entries and applies the rule above to every
file, so shrinking a listed file never needs a baseline edit. A sync (or a rollup carrying one)
regenerates the file with tools/sync/write_file_size_baseline.py instead of editing it by hand.

Inline test modules don't count, on the base or now: a normal build doesn't compile them. A test module is a
`#[cfg(test)]` line directly followed by `mod name {` (or `pub mod`, `pub(crate) mod`...) at the same indentation,
through its closing `}`. Other `#[cfg(test)]` items count. When in doubt the module counts, so the rule can't skip
real code. A small lexer tells code from strings, raw strings, chars and (nested) comments, and two readings of the
end must agree:
- braces: the module ends on the line where the braces in its code close;
- layout: rustfmt indents everything in the module deeper than `mod` and puts the closing `}` alone at its
  indentation, so the first later line that starts in code at that indentation or less must be that bare `}`.
Unformatted code, or a lexer mistake, makes them disagree, and the module counts. The two start lines must begin in
code too, so Rust source quoted in a string is no module.

    python3 tools/check_file_size.py --base origin/main
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
LIMIT = 3000
DOC = "docs/layers.md#file-size"
EXEMPT_PREFIXES = ("third_party/",)
CFG_TEST = re.compile(r"([ \t]*)#\[cfg\(test\)\]\s*")
TEST_MOD = re.compile(r"([ \t]*)(?:pub(?:\([a-z]+\))? )?mod \w+ \{\s*")
# Where code may stop: a newline, a comment, a raw string (with its b or c prefix), a string, a quote.
TOKEN = re.compile(r"\n|//|/\*|(?<!\w)[bc]?r(#*)\"|\"|'")
COMMENT_EDGE = re.compile(r"/\*|\*/")
BRACE = re.compile(r"[{}]")
STRING_REST = re.compile(r'(?:[^"\\]|\\.)*"', re.S)


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, encoding="utf-8", check=True, cwd=ROOT).stdout


def lines(text):
    return text.count("\n") + (1 if text and not text.endswith("\n") else 0)


def lex(text):
    """For each line of `text`: whether it starts in code rather than inside a string or a comment, the `{` minus the
    `}` in its code, and the lowest that count reaches along the line (`} mod live {` is 0 but dips to -1)."""
    starts, braces, lows, pos, end = [True], [0], [0], 0, len(text)

    def code(stop):
        for brace in BRACE.finditer(text, pos, stop):
            braces[-1] += 1 if brace.group() == "{" else -1
            lows[-1] = min(lows[-1], braces[-1])

    def skip_to(stop):
        newlines = text.count("\n", pos, stop)
        starts.extend([False] * newlines)
        braces.extend([0] * newlines)
        lows.extend([0] * newlines)
        return stop

    while token := TOKEN.search(text, pos):
        code(token.start())
        kind, pos = token.group(), token.end()
        if kind == "\n":
            starts.append(True)
            braces.append(0)
            lows.append(0)
        elif kind == "//":
            stop = text.find("\n", pos)
            pos = end if stop < 0 else stop
        elif kind == "/*":
            depth, stop = 1, pos
            while depth and (edge := COMMENT_EDGE.search(text, stop)):
                depth, stop = depth + (1 if edge.group() == "/*" else -1), edge.end()
            pos = skip_to(end if depth else stop)
        elif kind == "'":
            # A char literal ('x', '{', '\'', '\u{7b}') is skipped; a lifetime or label ('a) is not.
            if text.startswith("\\", pos):
                stop = text.find("'", pos + 2)
                pos = end if stop < 0 else stop + 1
            elif text.startswith("'", pos + 1):
                pos += 2
        elif kind == '"':
            rest = STRING_REST.match(text, pos)
            pos = skip_to(rest.end() if rest else end)
        else:
            stop = text.find('"' + token.group(1), pos)
            pos = skip_to(end if stop < 0 else stop + 1 + len(token.group(1)))
    code(end)
    return starts, braces, lows


def test_module_end(rows, starts, braces, lows, i):
    """The last line of the test module whose `#[cfg(test)]` is on line `i`, or None where the readings disagree."""
    indent, depth = TEST_MOD.fullmatch(rows[i + 1]).group(1), 1  # `mod name {` opens the one brace
    for j in range(i + 2, len(rows)):
        closed = depth + lows[j] <= 0
        depth += braces[j]
        if starts[j] and rows[j].strip() and len(rows[j]) - len(rows[j].lstrip()) <= len(indent):
            return j if depth == 0 and rows[j].rstrip() == indent + "}" else None
        if closed:
            return None
    return None


def test_module_lines(text):
    """How many lines of `text` are inline `#[cfg(test)] mod name { ... }` modules (see the module docstring)."""
    rows, (starts, braces, lows) = text.split("\n"), lex(text)
    skipped, i = 0, 0
    while i + 1 < len(rows):
        cfg, module = CFG_TEST.fullmatch(rows[i]), TEST_MOD.fullmatch(rows[i + 1])
        if cfg and module and cfg.group(1) == module.group(1) and starts[i] and starts[i + 1]:
            end = test_module_end(rows, starts, braces, lows, i)
            if end is not None:
                skipped += end - i + 1
                i = end + 1
                continue
        i += 1
    return skipped


def code_lines(text):
    """The lines that count toward the limit: all but inline test modules."""
    return lines(text) - test_module_lines(text)


def generated(text):
    return "@generated" in "\n".join(text.split("\n", 5)[:5])


def changes(base):
    """(path, old path or None) for each Rust file added, changed or renamed since the merge base with `base`,
    working tree included."""
    merge_base = git("merge-base", base, "HEAD").strip()
    out = []
    for line in git("diff", "--name-status", "-M", "--diff-filter=AMR", merge_base, "--", "*.rs").splitlines():
        parts = line.split("\t")
        status = parts[0]
        if status.startswith("R"):
            out.append((parts[2], parts[1]))
        elif status == "A":
            out.append((parts[1], None))
        else:
            out.append((parts[1], parts[1]))
    return merge_base, out


def parse_baseline(text):
    """Validate the committed boundary once; return its sync declaration and sizes."""
    marker, sizes = None, {}
    for line in text.splitlines():
        if line.startswith("# sync-baseline:"):
            value = line.removeprefix("# sync-baseline:").strip()
            if marker is not None or not re.fullmatch(r"[0-9a-f]{40}", value):
                raise ValueError("baseline needs one full upstream SHA in its sync-baseline header")
            marker = value
        elif line.strip() and not line.startswith("#"):
            path, raw_size = line.split()
            size = int(raw_size)
            if path in sizes or not path.endswith(".rs") or size <= 0:
                raise ValueError(f"invalid file-size baseline entry: {line}")
            sizes[path] = size
    return marker, sizes


def baseline_problems(sizes, previous, counts, sync):
    out = []
    for path, size in sizes.items():
        if size > previous.get(path, 0) and not sync:
            out.append(f"{path}: baseline added or raised without a changed sync-baseline upstream SHA")
        if path not in counts or size > counts[path]:
            out.append(f"{path}: stale-high baseline {size}; lower it to the current size or remove it")
    return out


def raised_problems(sizes, previous):
    """Outside a sync the entries are inert, but none may be added or raised: only a sync declares restoration."""
    return [f"{path}: baseline added or raised without a changed sync-baseline upstream SHA"
            for path, size in sizes.items() if size > previous.get(path, 0)]


def sync_provenance(marker, base, head="HEAD"):
    """Require a freshly verified upstream-main ancestor and its exact merge parent."""
    shallow = git("rev-parse", "--is-shallow-repository").strip()
    if shallow == "true":
        return ["sync baseline provenance needs full history; fetch --unshallow before checking"]
    if marker is None:
        return ["sync baseline provenance needs a sync-baseline header"]
    ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", marker, base], cwd=ROOT,
                              capture_output=True)
    if ancestor.returncode == 0:
        return ["sync baseline SHA is already an ancestor of the base"]
    upstream_url = os.environ.get("HORIZON_UPSTREAM_URL", "https://github.com/horizon-rust/horizon.git")
    try:
        fetched = subprocess.run(
            ["git", "fetch", "--no-tags", "--filter=blob:none", upstream_url,
             "+refs/heads/main:refs/sync/upstream-main"],
            cwd=ROOT, capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return ["cannot verify sync baseline: upstream main fetch timed out; network access is required"]
    if fetched.returncode != 0:
        return ["cannot verify sync baseline: upstream main fetch failed; network access is required: "
                + fetched.stderr.strip()]
    upstream_ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", marker, "refs/sync/upstream-main"],
        cwd=ROOT, capture_output=True)
    if upstream_ancestor.returncode != 0:
        return ["sync baseline SHA is not an ancestor of upstream main"]
    merges = git("rev-list", "--merges", "--parents", f"{base}..{head}")
    if not any(marker in row.split()[1:] for row in merges.splitlines()):
        return ["sync baseline SHA must be a parent of a merge commit in base..HEAD"]
    return []


def problems(entries, limit=LIMIT, baseline=None):
    """entries: (path, base line count, line count now, generated). The problem lines."""
    out = []
    for path, before, now, is_generated in entries:
        if path.startswith(EXEMPT_PREFIXES) or is_generated:
            continue
        if baseline is not None and path in baseline:
            if now > baseline[path]:
                out.append(f"{path}: {now} lines exceeds its sync baseline {baseline[path]}")
            continue
        if now > limit and now > before:
            was = f"{before} on the base" if before else "a new file"
            tests = path.rsplit("/", 1)[-1].removesuffix(".rs") + "_tests.rs"
            out.append(f"{path}: {now} lines without test modules ({was}). A file over {limit} lines may not grow. "
                       f"New tests go into a sibling {tests} (`#[path = \"{tests}\"] mod tests;`); code moves out "
                       f"to a new module in a pure-move PR first, not inside a fix ({DOC})")
    return out


def verified_sync_config(base, config_path):
    """Read an upstream inventory only after verifying a changed sync declaration."""
    baseline = ROOT / "tools/file_size_baseline.txt"
    marker, _ = parse_baseline(baseline.read_text() if baseline.exists() else "")
    previous = subprocess.run(["git", "show", f"{base}:tools/file_size_baseline.txt"],
                              cwd=ROOT, capture_output=True, text=True)
    old_marker, _ = parse_baseline(previous.stdout if previous.returncode == 0 else "")
    if marker is None or marker == old_marker:
        return None, []
    merge_base = git("merge-base", base, "HEAD").strip()
    errors = sync_provenance(marker, merge_base)
    if errors:
        return None, errors
    source = subprocess.run(["git", "show", f"{marker}:{config_path}"], cwd=ROOT,
                            capture_output=True, text=True)
    if source.returncode != 0:
        return None, [f"verified upstream commit has no {config_path} inventory"]
    config = json.loads(source.stdout)
    return config, []


def scan(base):
    """(merge base, entries): for each Rust file changed since the merge base with `base`, its path, code lines on the
    merge base, code lines now and whether it is generated. The input of `problems`."""
    merge_base, changed = changes(base)
    entries = []
    for path, old in changed:
        file = ROOT / path
        if not file.exists():
            continue
        text = file.read_text(encoding="utf-8", errors="replace")
        before = code_lines(git("show", f"{merge_base}:{old}")) if old else 0
        entries.append((path, before, code_lines(text), generated(text)))
    return merge_base, entries


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="the git ref the branch starts from (CI: the pull request's base)")
    args = parser.parse_args(argv)
    if not args.base:
        print("File size: no base to compare with, nothing checked.")
        return 0
    if git("rev-parse", "--is-shallow-repository").strip() == "true":
        print("File size check failed: provenance needs full history; fetch --unshallow before checking.")
        return 1
    merge_base, entries = scan(args.base)
    baseline_path = "tools/file_size_baseline.txt"
    file = ROOT / baseline_path
    marker, sizes = parse_baseline(file.read_text() if file.exists() else "")
    old = subprocess.run(["git", "show", f"{merge_base}:{baseline_path}"], cwd=ROOT,
                         capture_output=True, encoding="utf-8")
    previous_marker, previous = parse_baseline(old.stdout if old.returncode == 0 else "")
    declared_sync = marker is not None and marker != previous_marker
    if declared_sync:
        counts = {path: code_lines((ROOT / path).read_text()) for path in sizes if (ROOT / path).is_file()}
        provenance = sync_provenance(marker, merge_base)
        found = provenance + baseline_problems(sizes, previous, counts, not provenance)
        caps = sizes
    else:
        found = raised_problems(sizes, previous)
        caps = None
    found.extend(problems(entries, baseline=caps))
    if found:
        print(f"File size check failed ({DOC}):")
        for problem in found:
            print(f"  {problem}")
        return 1
    kept = f"{len(sizes)} sync baselines" if declared_sync else "no sync declared, baselines not used"
    print(f"File size check passed: {len(entries)} changed Rust files, {kept}; none over {LIMIT} lines grew beyond "
          f"{'their cap' if declared_sync else 'the merge base'} (inline test modules not counted).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
