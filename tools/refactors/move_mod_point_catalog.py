#!/usr/bin/env python3
"""Move the unchanged mod-point catalog values below the package/build-tool boundary."""
import argparse
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/horizon-mod-package/src/manifest/inject/mod_points.rs"
DEST = ROOT / "crates/horizon-mod-point-shape/src/catalog.rs"
LIB = ROOT / "crates/horizon-mod-point-shape/src/lib.rs"
HASH = "fa2dd076fa55321e6137c39b7e90888e1176a19925403518022f8f8207a65337"
HEADER = "//! Serialized values shared by mod-point generation and package validation.\nuse serde::{Deserialize, Serialize};\n\n"
MODULE = "pub mod catalog;\n\n"
IMPORT = "use point_shape::catalog::{Catalog, Point};\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    source = SOURCE.read_text()
    if args.verify:
        assert hashlib.sha256(DEST.read_text().removeprefix(HEADER).split("impl Catalog {", 1)[0].encode()).hexdigest() == HASH
        assert "pub struct Catalog" not in source and "pub struct Point" not in source
        assert IMPORT in source and MODULE in LIB.read_text()
        return
    start = source.index("/// A mod crate's points")
    end = source.index("/// An installed library")
    block = source[start:end]
    assert hashlib.sha256(block.encode()).hexdigest() == HASH
    assert not DEST.exists()
    DEST.write_text(HEADER + block)
    SOURCE.write_text((source[:start] + source[end:]).replace(
        "use serde::{Deserialize, Serialize};\n", IMPORT))
    lib = LIB.read_text()
    marker = "use proc_macro2::{Span, TokenStream};"
    assert lib.count(marker) == 1
    LIB.write_text(lib.replace(marker, MODULE + marker))


if __name__ == "__main__":
    main()
