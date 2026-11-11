#!/usr/bin/env python3
"""Move the launcher's platform asset table verbatim out of the large backend."""
from pathlib import Path


def main():
    root = Path(__file__).resolve().parents[2]
    backend = root / "launcher/src/backend.rs"
    destination = root / "launcher/src/backend/release_assets.rs"
    source = backend.read_text()
    start = source.index("/// The release assets of each system Horizon ships for:")
    end = source.index("/// `a == b`, in a const context.", start)
    declaration = source[start:end]
    if destination.exists():
        raise SystemExit(f"Refusing to overwrite {destination}")
    destination.write_text(declaration)
    backend.write_text(source[:start] + "mod release_assets;\npub use release_assets::PLATFORMS;\n\n" + source[end:])


if __name__ == "__main__":
    main()
