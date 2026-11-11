#!/usr/bin/env python3
"""mdBook preprocessor: render generated chapters (catalog notes, wasm boundary coverage)
without writing a generated source page."""
import json
from pathlib import Path
import subprocess
import sys

PAGE = "reference/hook-notes.md"
COVERAGE_PAGE = "wasm-coverage.md"


def expand(book, content, title, path):
    """Fill the draft chapter from SUMMARY, retaining its published URL and position."""
    found = []

    def visit(items):
        for item in items:
            if "Chapter" not in item:
                continue
            chapter = item["Chapter"]
            if chapter["name"] == title and chapter["path"] is None:
                chapter.update(content=content, path=path, source_path=None)
                found.append(chapter)
            visit(chapter["sub_items"])

    visit(book["sections"])
    assert len(found) == 1, f"SUMMARY must contain one draft [{title}]() chapter"
    return book


def generated(context, chapter):
    """The `cargo run -p xtask -- mod-api <chapter>` output, from mdBook's root."""
    root = Path(context["root"]).resolve().parents[1]
    return subprocess.run(
        ["cargo", "run", "--quiet", "-p", "xtask", "--", "mod-api", chapter],
        cwd=root, check=True, stdout=subprocess.PIPE, text=True,
    ).stdout


def preprocess(context, book):
    return expand(book, generated(context, "notes"), "Hook notes", PAGE)


def coverage(context, book):
    return expand(book, generated(context, "coverage"), "Wasm boundary coverage", COVERAGE_PAGE)


if __name__ == "__main__" and sys.argv[1:2] != ["supports"]:
    print(json.dumps(preprocess(*json.load(sys.stdin))))
