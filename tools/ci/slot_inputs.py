"""Derive source-stage inputs from listed callers, not a hand-kept source list."""
import tomllib
import re

SLOTS = "crates/horizon-mod-api-gen/slots/"


def declarations(text):
    return {tuple(part.strip() for part in line.split("|")[:3])
            for line in text.splitlines() if line.strip() and not line.lstrip().startswith("#")}


def rows(root):
    return set().union(*(declarations(path.read_text()) for path in (root / SLOTS).glob("*.txt")))


def inputs(root):
    paths = {"tools/ci/stage_slots.py", "tools/ci/slot_inputs.py", "tools/prebuilt_game.py",
             ".github/workflows/dyna-ci.yml"}
    for target, *_ in rows(root):
        parts = target.split("::")
        crate = root / "crates" / parts[0].replace("_", "-")
        manifest = crate / "Cargo.toml"
        paths.add(manifest.relative_to(root).as_posix())
        package = tomllib.loads(manifest.read_text())
        source = (crate / package.get("lib", {}).get("path", "src/lib.rs")).resolve().parent
        # The final module precedes the function and optional impl receiver. Game's
        # area directories are physical only; recursive lookup handles those too.
        modules = parts[1:-1]
        modules = [part for part in modules if not part[:1].isupper()]
        matches = []
        while modules and not matches:
            name = modules.pop()
            matches = list(source.rglob(f"{name}.rs")) + list(source.rglob(f"{name}/mod.rs"))
        if not matches:
            # Inline modules have no own file. Conservatively select the crate.
            paths.add(source.relative_to(root).as_posix() + "/")
        pending = list(matches)
        visited = set()
        while pending:
            path = pending.pop().resolve()
            if path in visited:
                continue
            visited.add(path)
            # Explicit module paths need not resemble their logical target names.
            for name in re.findall(r'#\[path\s*=\s*"([^"]+)"\]', path.read_text()):
                child = path.parent / name
                if child.is_file():
                    pending.append(child)
        paths.update(path.relative_to(root).as_posix() for path in visited)
    return paths


def applies(root, changed):
    selected = inputs(root)
    return any(path.startswith(SLOTS) or path in selected or
               any(item.endswith("/") and path.startswith(item) for item in selected)
               for path in changed)

