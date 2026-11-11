#!/usr/bin/env python3
"""Move Wire call-table role consumers behind discovery accessors.

The legacy tables remain unchanged. New traits use per-item role files instead
of appending to a shared import/export list. Run once from the repository root.
"""
from pathlib import Path

for name in ["calls.rs", "host.rs"]:
    path = Path("crates/horizon-mod-api-gen/src/wire") / name
    source = path.read_text()
    assert "super::IMPORTS" in source
    path.write_text(source.replace("super::IMPORTS", "super::imports()"))

path = Path("crates/horizon-mod-api-gen/src/wire/mod.rs")
source = path.read_text()
source = source.replace("mod ty;", "mod ty;\nmod trait_roles;")
old = "pub(super) fn exports() -> &'static [&'static str] {\n    EXPORTED.get().map_or(EXPORTS, Vec::as_slice)\n}"
assert source.count(old) == 1
source = source.replace(old, "pub(super) fn exports() -> &'static [&'static str] {\n    trait_roles::exports()\n}\n\n/// All imported traits, including discovered per-item declarations.\npub(super) fn imports() -> &'static [&'static str] {\n    trait_roles::imports()\n}")
source = source.replace("static EXPORTED: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();\n", "")
old = "    let mut exported: Vec<&'static str> = EXPORTS.to_vec();"
assert source.count(old) == 1
source = source.replace(old, "    let (imported, mut exported, paths) = trait_roles::declared(root)?;\n    types.index.read.borrow_mut().extend(paths);")
source = source.replace("exported.push(Box::leak(owner.into_boxed_str()));", "exported.push(owner);")
source = source.replace("    let _ = EXPORTED.set(exported);", "    trait_roles::select(imported, exported);")
source = source.replace("for owner in IMPORTS.iter().chain(exports())", "for owner in imports().iter().chain(exports())")
source = source.replace("&& IMPORTS.contains(&method.owner)", "&& imports().contains(&method.owner)")
source = source.replace("if IMPORTS.iter().any(", "if imports().iter().any(")
path.write_text(source)
