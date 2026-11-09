#!/usr/bin/env python3
"""Run only after other builds finish: mutate the converter, test the real hook,
restore exact bytes, then rerun the baseline. Does not run workspace tests.
"""
from pathlib import Path
import json
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "crates/horizon-client/src/json_nbt.rs"
OUT = ROOT / "target/crafting-metadata-proof"
COMMAND = ["cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), "--lib",
           "ingredient_"]


def prepare():
    # Compile the exact engine ingredient reader and its exact hook fixture,
    # without rebuilding graphics for each converter mutant. The load hook
    # uses real mod_api dispatch and real LoadContext; only the unexercised
    # engine-stack match callback is a trap, never a simulated result.
    (OUT / "src").mkdir(parents=True, exist_ok=True)
    deps = {"mod_api": "horizon-mod-api", "horizon-world": "horizon-world",
            "horizon-items": "horizon-items", "horizon-core": "horizon-core"}
    manifest = '[package]\nname = "crafting-metadata-proof"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\nserde_json = "1.0.151"\nserde = { version = "1", features = ["derive"] }\n'
    for alias, package in deps.items():
        features = {"horizon-mod-api": ["testing"], "horizon-items": ["test-support"]}.get(package, [])
        manifest += f'{json.dumps(alias)} = {{ package = {json.dumps(package)}, path = {json.dumps(str(ROOT / "crates" / package))}, features = {json.dumps(features)} }}\n'
    profiles = (ROOT / "Cargo.toml").read_text().split("[profile.dev]", 1)[1].split("[profile.release]", 1)[0]
    manifest += "\n[profile.dev]" + profiles
    (OUT / "Cargo.toml").write_text(manifest)
    shutil.copyfile(ROOT / "Cargo.lock", OUT / "Cargo.lock")
    (OUT / "src/lib.rs").write_text(
        'pub use horizon_items::items::{ItemStack, Items};\n'
        f'#[path = {json.dumps(str(SOURCE))}]\npub mod json_nbt;\n'
        f'#[path = {json.dumps(str(ROOT / "crates/horizon-client/src/crafting/ingredient.rs"))}]\npub mod ingredient;\n'
        f'#[cfg(test)] mod form_fixture {{ pub use mod_api::item::crafting::*; #[path = {json.dumps(str(ROOT / "crates/horizon-mod-api/src/crafting/ingredient_form_tests.rs"))}] mod tests; }}\n'
        'mod native_stack_hooks {\n'
        'pub fn ingredient_test(_: &horizon_world::native_load_context::LoopContext, _: &str, _: &horizon_core::nbt::Compound, _: &horizon_items::items::ItemStack, _: &mut bool) { panic!("match callback is not exercised by the load-boundary fixture"); }\n}\n'
    )


def main():
    prepare()
    subprocess.run(COMMAND, cwd=ROOT, check=True)
    original = SOURCE.read_text()
    mutations = [
        ("int width", "return Tag::Int(value);", "return Tag::Short(value as i16);"),
        ("long width", "return Tag::Long(value);", "return Tag::Int(value as i32);"),
        ("list kind", "Value::Array(values) => Tag::List(", "Value::Array(values) if values.iter().all(Value::is_number) => Tag::ByteArray(vec![]),\n        Value::Array(values) => Tag::List("),
        ("nested compound", "Value::Object(values) => Tag::Compound(", "Value::Object(_) if nested => Tag::compound([] as [(&str, Tag); 0]),\n        Value::Object(values) => Tag::Compound("),
    ]
    try:
        for name, old, new in mutations:
            assert original.count(old) == 1, name
            SOURCE.write_text(original.replace(old, new))
            result = subprocess.run(COMMAND, cwd=ROOT, capture_output=True, text=True)
            if result.returncode != 101 or "test result: FAILED" not in result.stdout:
                raise AssertionError(f"Mutation survived or failed to compile: {name}\n{result.stdout}\n{result.stderr}")
            print(f"Killed by the real ingredient hook test: {name}", flush=True)
    finally:
        SOURCE.write_text(original)
    subprocess.run(COMMAND, cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
