#!/usr/bin/env python3
"""Rerun the eight review mutants; only an executed failing Rust test kills one."""
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[3]
rt = root / "crates/horizon-mod-runtime/src/registry_keys.rs"
api = root / "crates/horizon-mod-api/src/content/registry_keys.rs"
game = root / "crates/horizon-client/src/game.rs"
original = {p: p.read_text() for p in (rt, api, game)}
lookup = "Some(BlockKey(*resolved_block_keys().get()?.slots.get(id)?))"
cases = [
    ("skip_air", rt, "for id in all {", 'for id in all { if id == "minecraft:air" { continue; }'),
    ("off_by_one", api, lookup, "Some(BlockKey(*resolved_block_keys().get()?.slots.get(id)? + 1))"),
    ("reverse_order", rt, "for id in all {", "for id in all.into_iter().rev() {"),
    ("skip_mod_names", rt, "for id in all {", 'for id in all { if !id.starts_with("minecraft:") { continue; }'),
    ("unknown_defaults", api, lookup, "Some(BlockKey(*resolved_block_keys().get()?.slots.get(id).unwrap_or(&0)))"),
    ("per_call_clone", api, lookup, "Some(BlockKey(*resolved_block_keys().get()?.clone().slots.get(id)?))"),
    ("omit_load_boundary", game, "mod_api::__private::resolve_block_keys((0..registry.len()).map(|index| {", "mod_api::__private::resolve_block_keys((0..0).map(|index| {"),
    ("accept_unknown_requests", rt, "self.resolve(|id| all.contains(id).then_some(()), unknown)?;", "let _ = self.resolve(|id| all.contains(id).then_some(()), unknown);"),
]
out = root / "target/api-pr3b-fixups/mutations"
out.mkdir(parents=True, exist_ok=True)
env = dict(os.environ, CARGO_TARGET_DIR=str(root / "target"))
results = []
try:
    for name, path, old, new in cases:
        assert old in original[path], name
        path.write_text(original[path].replace(old, new, 1))
        commands = [
            ["cargo", "test", "-p", "horizon-mod-api", "--lib", "completed_blocks_preserve_slots_and_bind_unrequested_states"],
            ["python3", "tools/refactors/api-pr3b/test_block_key_load_boundary.py"],
        ]
        killed = False
        for index, command in enumerate(commands):
            run = subprocess.run(command, cwd=root, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (out / f"{name}-{index}.log").write_text(run.stdout)
            if run.returncode:
                killed = "test result: FAILED" in run.stdout and "error[E" not in run.stdout
                break
        results.append({"mutant": name, "killed": killed, "test": index})
        print(results[-1], flush=True)
        path.write_text(original[path])
finally:
    for path, text in original.items():
        path.write_text(text)
    (out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
assert len(results) == len(cases) and all(r["killed"] for r in results), results
