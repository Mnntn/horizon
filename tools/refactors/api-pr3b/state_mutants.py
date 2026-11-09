#!/usr/bin/env python3
"""Kill owned-state facade/transport/reload mutants using the production focused tests."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(root / "tools"))
import lab  # noqa: E402

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--group", choices=["state", "range", "write"], default="state")
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
facade = root / "crates/horizon-mod-api/src/block/block_state.rs"
values = root / "crates/horizon-client/src/game/modding/api_values.rs"
renderer = root / "crates/horizon-client-render/src/mesher/mod_tint.rs"
service = ["cargo", "test", "-p", "horizon-mod-api", "--lib", "metadata_service_reads_edits_and_refuses_invalid_transport"]
worker = ["cargo", "test", "-p", "horizon-mod-api", "--test", "block_metadata", "metadata_resource_worker_reads_allocate_nothing"]
tint = ["cargo", "test", "-p", "horizon-client-render", "--lib", "mesher::mod_tint_tests::"]
cases = [
    ("property_loses_name", facade, "metadata.property((&self).into(), name)", 'metadata.property((&self).into(), "absent")', service),
    ("with_returns_original", facade, "metadata.with((&self).into(), name, value).map(Into::into)", "{ let _ = (metadata, name, value); Ok(self) }", service),
    ("transport_loses_ordinal", facade, "state: value.state,", "state: 0,", service),
    ("property_allocates", facade, "metadata.property((&self).into(), name)", "{ std::hint::black_box(name.to_owned()); metadata.property((&self).into(), name) }", worker),
    ("reload_uses_default_not_properties", values, "mod_api::BlockState::of(block_metadata_table()?, block, &properties)", "mod_api::BlockState::of(block_metadata_table()?, block, { let _ = &properties; &[] })", ["python3", "tools/refactors/api-pr3b/test_block_key_load_boundary.py"]),
    ("source_without_metadata_commits", renderer, "let sources = has_custom_sources(by_block);", "let sources = false; let _ = has_custom_sources(by_block);", tint),
    ("region_returns_air_for_every_state", renderer, "*states\n            .get(self.state_id(pos) as usize)\n            .unwrap_or(&states[0])", "{ let _ = pos; states[0] }", tint),
]
if args.group == "range":
    span = root / "crates/horizon-mod-api/src/block/block_state_range.rs"
    definitions = root / "crates/horizon-mod-runtime/src/block_definitions.rs"
    focused = ["cargo", "test", "-p", "horizon-mod-api", "--lib", "local_range_validates_transport_and_never_wraps_ordinals"]
    cases = [
        ("span_accepts_overflow", span, "if count > u16::MAX as u32 + 1 - first.ordinal() as u32 {", "if false {", focused),
        ("membership_ignores_key", span, "state.block().0 == self.first.block().0", "true", focused),
        ("iteration_ignores_start", span, "(self.first.ordinal() as u32 + offset) as u16", "offset as u16", focused),
        ("transport_skips_span_check", span, "Self::new(value.first, value.count)", "Ok(Self { first: value.first, count: value.count })", focused),
        ("metadata_drops_last_state", definitions, "self.states.len() as u32", "self.states.len().saturating_sub(1) as u32", service),
    ]
if args.group == "write":
    native = root / "crates/horizon-client/src/game/modding/native_entity_refs.rs"
    errors = root / "crates/horizon-mod-api/src/world/level_write_error.rs"
    generator = root / "crates/horizon-mod-api-gen/src/host.rs"
    focused = ["cargo", "test", "-p", "horizon-client", "--lib", "level_block_writes_validate_and_change_world"]
    cases = [
        ("write_ignores_residency", native, "let world = self.native_level_world_result(level)?;", "let world = self.native_level_world_result(level).unwrap_or(&self.world);", focused),
        ("write_disables_phase_refusal", generator, "if !mod_api::__private::access_allowed({name:?}", "if false && !mod_api::__private::access_allowed({name:?}", focused),
        ("write_collapses_invalid_state", errors, "error => Self::InvalidState(error),", "_ => Self::Unavailable,", focused),
        ("write_claims_changed_without_mutation", native, "Ok(self.native_set_block(level.dimension_id(), pos, state, flags.bits()))", "Ok(true)", focused),
    ]
results = []
for name, path, old, new, command in cases:
    original = path.read_text()
    if original.count(old) != 1:
        raise RuntimeError(f"{name}: mutation target is not unique")
    try:
        path.write_text(original.replace(old, new))
        env = dict(os.environ, CARGO_TARGET_DIR=str(root / "target"), CARGO_BUILD_JOBS="4", HORIZON_TEST_RESOURCES=str(root / ".local/horizon-client/26.3"))
        if args.group == "write":
            # Build from the mutated production source, then run the exact game test outside
            # the checkout, never in a player's home or the prepared resource directory.
            build = subprocess.run(command + ["--no-run", "--message-format=json"], cwd=root, env=env, capture_output=True, text=True, timeout=600)
            if build.returncode:
                run = subprocess.CompletedProcess(command, build.returncode, build.stdout + build.stderr)
            else:
                artifacts = [json.loads(line) for line in build.stdout.splitlines()]
                binaries = [row["executable"] for row in artifacts if row.get("reason") == "compiler-artifact" and row.get("target", {}).get("name") == "horizon_client" and row.get("executable")]
                if len(binaries) != 1:
                    raise RuntimeError(f"unexpected test executables: {binaries}")
                scratch = Path(tempfile.mkdtemp(prefix="write-mutant-", dir=root / "target"))
                for key in ["HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME", "HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"]:
                    home = scratch / key
                    home.mkdir()
                    env[key] = str(home)
                env = lab.game_env(env)
                run = subprocess.run([binaries[0], command[-1], "--ignored", "--test-threads=1", "--nocapture"], cwd=scratch, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=540)
        else:
            run = subprocess.run(command, cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600)
        (args.output / f"{name}.log").write_text(run.stdout)
        killed = run.returncode != 0 and "panicked at" in run.stdout and ("assertion" in run.stdout or "no face tinted" in run.stdout) and "test result: FAILED." in run.stdout
        results.append({"name": name, "killed_by_assertion": killed, "exit_code": run.returncode})
        print(json.dumps(results[-1]), flush=True)
        if not killed:
            raise RuntimeError(f"{name}: survived or failed without an executed assertion; see log")
    finally:
        path.write_text(original)
(args.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
