"""Focused discovery, published-anchor and relocation regressions for the book."""
import copy
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

with patch.object(sys, "path", [*sys.path, str(Path(__file__).resolve().parents[1] / "tools")]):
    import modding_concept_pages as pages
    import split_modding_concepts as move


class ConceptPages(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = Path(self.tmp.name)
        self.book = {"sections": []}
        for part, group in zip(pages.FOLDERS, pages.GROUPS):
            self.book["sections"].append({"PartTitle": part})
            path = self.src / group
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("# Original\n\n" + pages.MARKER + "\n")
            directory = path.with_suffix("")
            directory.mkdir()
            (directory / "legacy-anchors.txt").write_text("original\nfirst\nsecond\n")
            (directory / "000-first.md").write_text(
                "<!-- chapter: First -->\n\n## First\n\n[second](001-second.md#second)\n\n")
            (directory / "001-second.md").write_text("<!-- chapter: Second -->\n\n### Second\n\nPayload.\n")
            self.book["sections"].append({"Chapter": dict(name="Original", path=group,
                content=path.read_text(), number=[1], parent_names=[], sub_items=[])})

    def test_import_does_not_change_discovery_search_paths(self):
        spec = importlib.util.spec_from_file_location("concept_import_probe", __file__)
        with patch.object(sys, "path", list(sys.path)):
            before = list(sys.path)
            spec.loader.exec_module(importlib.util.module_from_spec(spec))
            self.assertEqual(sys.path, before)

    def test_new_concept_needs_no_shared_list_edit(self):
        directory = (self.src / pages.GROUPS[0]).with_suffix("")
        (directory / "002-third.md").write_text("<!-- chapter: Third -->\n\nA new concept.\n")
        result = pages.expand(copy.deepcopy(self.book), self.src)
        chapter = result["sections"][1]["Chapter"]
        self.assertEqual([c["Chapter"]["name"] for c in chapter["sub_items"]], ["First", "Second", "Third"])
        self.assertIn("[second](#second)", chapter["content"])
        self.assertIn("# Third", chapter["sub_items"][2]["Chapter"]["content"])

    def test_unlisted_page_joins_its_part_by_title_and_is_numbered(self):
        (self.src / "concepts/zeta.md").write_text("# Zeta\n\nLast by title.\n")
        (self.src / "concepts/alpha.md").write_text("# Alpha\n\n[first](trait-mods/000-first.md#first)\n")
        result = pages.expand(copy.deepcopy(self.book), self.src)
        flat = [(i.get("PartTitle"), i.get("Chapter", {}).get("name"), i.get("Chapter", {}).get("number"))
                for i in result["sections"]]
        self.assertEqual(flat, [("Concepts", None, None), (None, "Original", [1]), (None, "Alpha", [2]),
                                (None, "Zeta", [3]), ("How-to", None, None), (None, "Original", [4])])
        self.assertEqual(result["sections"][2]["Chapter"]["path"], "concepts/alpha.md")
        self.assertEqual(result["sections"][5]["Chapter"]["sub_items"][1]["Chapter"]["number"], [4, 2])

    def test_equal_titles_use_filename_order_not_directory_iteration_order(self):
        for name in ("zeta", "alpha"):
            (self.src / f"concepts/{name}.md").write_text("# Same title\n")
        glob = Path.glob
        def reversed_glob(path, *args, **kwargs):
            return iter(sorted(glob(path, *args, **kwargs), reverse=True))
        with patch.object(Path, "glob", reversed_glob):
            result = pages.expand(copy.deepcopy(self.book), self.src)
        found = [c["path"] for c in pages.chapters(result["sections"])
                 if c["path"] in {"concepts/alpha.md", "concepts/zeta.md"}]
        self.assertEqual(found, ["concepts/alpha.md", "concepts/zeta.md"])

    def test_discovered_page_links_are_checked(self):
        (self.src / "how-to/broken.md").write_text("# Broken\n\n[gone](missing.md)\n")
        with self.assertRaisesRegex(AssertionError, "how-to/broken.md: missing link target"):
            pages.expand(copy.deepcopy(self.book), self.src)

    def test_page_outside_the_discovered_folders_must_be_listed(self):
        (self.src / "reference").mkdir()
        (self.src / "reference/orphan.md").write_text("# Orphan\n")
        with self.assertRaisesRegex(AssertionError, "doesn't render.*reference/orphan.md"):
            pages.expand(copy.deepcopy(self.book), self.src)

    def test_links_resolve_generated_chapter_and_anchor_without_a_file(self):
        book = copy.deepcopy(self.book)
        book["sections"].append({"Chapter": dict(name="Hook notes", path="reference/hook-notes.md",
            content="# Hook notes\n\n## Example\n", sub_items=[])})
        path = (self.src / pages.GROUPS[0]).with_suffix("") / "000-first.md"
        path.write_text(path.read_text() + "[notes](../../reference/hook-notes.md#example)\n")
        pages.expand(copy.deepcopy(book), self.src)
        self.assertFalse((self.src / "reference/hook-notes.md").exists())
        path.write_text(path.read_text().replace("#example", "#absent"))
        with self.assertRaisesRegex(AssertionError, "missing anchor"):
            pages.expand(book, self.src)

    def test_unlinked_published_anchor_cannot_disappear(self):
        path = (self.src / pages.GROUPS[0]).with_suffix("") / "000-first.md"
        path.write_text(path.read_text().replace("## First", "## Renamed"))
        with self.assertRaisesRegex(AssertionError, "published anchors lost.*first"):
            pages.expand(self.book, self.src)

    def test_missing_relative_target_or_anchor_fails(self):
        path = (self.src / pages.GROUPS[0]).with_suffix("") / "000-first.md"
        original = path.read_text()
        for target, error in [("missing.md", "missing link target"),
                              ("001-second.md#absent", "missing anchor")]:
            path.write_text(original.replace("001-second.md#second", target))
            with self.assertRaisesRegex(AssertionError, error):
                pages.expand(copy.deepcopy(self.book), self.src)

    def test_reference_destinations_are_checked_and_relocated(self):
        path = "concepts/trait-mods/000-first.md"
        for target, error in [("missing.md", "missing link target"),
                              ("001-second.md#absent", "missing anchor")]:
            text = f"[x][ref]\n\n[ref]: <{target}>\n"
            with self.assertRaisesRegex(AssertionError, error):
                pages.validate(text, path, self.src, {})
        original = "[x][ref]\n\n[ref]: <#second> \"Title\"\n"
        transformed = pages.rewrite(original, lambda u: pages.relocate(u, "original.md", path,
            {("original.md", "second"): "concepts/trait-mods/001-second.md"}), lambda u: u)
        self.assertEqual(transformed, '[x][ref]\n\n[ref]: <001-second.md#second> "Title"\n')
        pages.validate(transformed, path, self.src, {})

    def test_source_root_relative_links_are_rejected(self):
        for url in ("/missing.html", "%2Fmissing.html"):
            for text in (f"[x]({url})", f"[x][ref]\n\n[ref]: {url}\n"):
                with self.assertRaisesRegex(AssertionError, "root-relative link"):
                    pages.validate(text, "concepts/trait-mods/000-first.md", self.src, {})

    def test_rendered_root_and_reference_destinations_are_checked(self):
        output = self.src / "html"
        path = (output / pages.GROUPS[0]).with_suffix(".html")
        path.parent.mkdir(parents=True)
        for href, error in [("/missing.html", "root-relative link"),
                            ("%2Fmissing.html", "root-relative link"),
                            ("missing.html", "missing rendered link"),
                            ("#absent", "missing rendered anchor")]:
            path.write_text(f'<a href="{href}">x</a>')
            with self.assertRaisesRegex(AssertionError, error):
                pages.check_html(output, self.src)

    def test_missing_compiled_include_anchor_fails(self):
        (self.src / "code.rs").write_text("// ANCHOR: present\n// ANCHOR_END: present\n")
        text = "{{#include ../../code.rs:missing}}"
        with self.assertRaisesRegex(AssertionError, "missing include anchor"):
            pages.validate(text, "concepts/trait-mods/000-first.md", self.src, {})

    def test_move_is_exact_and_rebases_cross_concept_links(self):
        original = "# Original\n\n## First\n\n[second](#second)\n\n## Second\n\nPayload.\n"
        with patch.object(move, "original", return_value=original):
            result = move.split("fixture")
        for name, text in result.items():
            path = self.src / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        for group in pages.GROUPS:
            self.assertEqual(pages.aggregate(self.src, group), original)


if __name__ == "__main__":
    unittest.main()
