#!/usr/bin/env python3
"""Benchmark the real generated use_item firing shim and checked-in conversion.

Creates an isolated Cargo project under target, never edits generated output or
an SDK source file. --mode engine is for the pre-3a baseline; --mode owned is for
3a. Both invoke the selected checkout's generator and one actual ItemPart.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def block(source, marker):
    start = source.index(marker)
    opening = source.index("{", start)
    depth = 0
    # Skip strings/comments while counting Rust block delimiters.
    tokens = re.finditer(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/|[{}]', source[opening:], re.S)
    for match in tokens:
        token = match.group()
        if token == "{":
            depth += 1
        elif token == "}":
            depth -= 1
            if not depth:
                return source[start:opening + match.end()]
    raise ValueError(f"unclosed Rust block: {marker}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--mode", choices=["engine", "owned"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root, output = args.repo.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    (output / "src").mkdir(exist_ok=True)
    template = Path(__file__).with_name("bench_native.rs").read_text()
    provenance = {}
    if args.mode == "owned":
        source = (root / "crates/horizon-client/src/game/modding/api_values.rs").read_text()
        markers = ["pub(in crate::game) trait TryIntoApi", "pub(in crate::game) enum StackReadError", "impl TryIntoApi for EngineStack"]
        chunks = [block(source, marker) for marker in markers]
        provenance = {marker: hashlib.sha256(chunk.encode()).hexdigest() for marker, chunk in zip(markers, chunks)}
        converter = "pub mod api_values { use horizon_items::items::ItemStack as EngineStack; use horizon_core::nbt::Tag; use mod_api::ItemStack as ApiStack;\n" + chunks[0] + "\n#[derive(Debug)]\n" + chunks[1] + "\n" + chunks[2] + "\n}\npub fn snapshot(stack: &EngineStack) -> mod_api::ItemStack { use api_values::TryIntoApi; stack.try_into_api().unwrap() }"
        template = template.replace("// COPIED_CONVERTER", converter)
        template = template.replace("// RESOLVE_ITEMS", 'mod_api::__private::resolve_items(["minecraft:stick", "minecraft:iron_sword"]).unwrap();')
        template = template.replace("// SNAPSHOT_REPORT", 'report(&format!("snapshot/{name}"), || { black_box(game::snapshot(black_box(stack))); });')
        template = template.replace("// API_REPORT", '''let api = game::snapshot(&plain);
    let key = api.item();
    report("construct/plain", || { black_box(mod_api::ItemStack::new(black_box(key), black_box(1)).unwrap()); });
    report("read/plain", || { black_box((black_box(&api).item() == key, black_box(&api).count())); });
    report("clone/plain", || { black_box(black_box(&api).clone()); });''')
        template = template.replace("// RECORD_SIZE", 'println!("SIZE,stack_record,{}", std::mem::size_of::<mod_api::item::item_stack::StackRecord>());')
    else:
        template = template.replace("// API_REPORT", '''report("construct/plain", || { black_box(EngineStack::new(black_box("minecraft:stick"), black_box(1))); });
    report("read/plain", || { black_box((black_box(&plain).item.as_ref() == "minecraft:stick", black_box(&plain).count)); });
    report("clone/plain", || { black_box(black_box(&plain).clone()); });''')
    (output / "src/main.rs").write_text(template)
    manifest = '''[package]
name = "api-stack-native-bench"
version = "0.0.0"
edition = "2024"
[workspace]
[features]
alloc-count = []
[dependencies]
'''
    for package in ["horizon-core", "horizon-items", "horizon-mod-api"]:
        key = "mod_api" if package == "horizon-mod-api" else package
        features = ', features = ["testing"]' if package == "horizon-mod-api" else ""
        manifest += f'{key} = {{ package = "{package}", path = {json.dumps(str(root / "crates" / package))}{features} }}\n'
    manifest += f'''[build-dependencies]
horizon-mod-api-gen = {{ path = {json.dumps(str(root / "crates/horizon-mod-api-gen"))} }}
[profile.release]
opt-level = 3
lto = false
codegen-units = 16
[profile.release.package."*"]
opt-level = 1
'''
    for package in ["horizon-core", "horizon-items", "horizon-mod-api", "horizon-mod-runtime"]:
        manifest += f'[profile.release.package."{package}"]\nopt-level = 3\n'
    (output / "Cargo.toml").write_text(manifest)
    (output / "build.rs").write_text('''fn main() {
    let root = std::path::PathBuf::from(std::env::var("BENCH_REPO").unwrap());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate(&root, &out, horizon_mod_api_gen::Target::Client).unwrap();
    let source = std::fs::read_to_string(out.join("mod_api_fire_generated.rs")).unwrap();
    let fire = source.lines().find(|line| line.contains("fn fire_use_item ") || line.contains("fn fire_use_item(")).expect("generated use_item shim");
    std::fs::write(out.join("bench_fire.rs"), fire).unwrap();
    println!("cargo:rerun-if-env-changed=BENCH_REPO");
}''')
    env = dict(os.environ, BENCH_REPO=str(root), CARGO_TARGET_DIR=str(Path(__file__).resolve().parents[3] / "target/api-pr3a-bench-build"))
    command = ["cargo", "run", "--quiet", "--release", "-j", "2", "--manifest-path", str(output / "Cargo.toml")]
    results = []
    for extra, label in [([], "time"), (["--features", "alloc-count"], "allocations")]:
        with (output / f"{label}.log").open("w") as log:
            run = subprocess.run(command + extra, env=env, text=True, stdout=subprocess.PIPE, stderr=log)
        (output / f"{label}.csv").write_text(run.stdout)
        if run.returncode:
            raise SystemExit(f"benchmark failed: {output / (label + '.log')}")
        results.extend(run.stdout.splitlines())
    report = {"head": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(), "mode": args.mode, "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "copied_source_sha256": provenance, "results": results, "note": "Timings use the system allocator; allocation counts are a separate build. Actual hot crates are opt-level 3; unused dependencies opt-level 1. Median of 9 x 200000 calls; one subscribed ItemPart observes the stack and increments one relaxed atomic counter."}
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
