#!/usr/bin/env python3
"""Run the exact worker adapters without linking the window/game executable.

Only module paths are rewired; production function bodies and unit tests are read
from the checkout. Mutation runs restore every source in finally before proceeding.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "target/structure-context-proof"
GAME = ROOT / "crates/horizon-client/src/game/modding"


def between(source, start, end):
    assert source.count(start) == 1, start
    begin = source.index(start)
    return source[begin:source.index(end, begin)]


def prepare():
    OUT.mkdir(parents=True, exist_ok=True)
    deps = {name: ROOT / "crates" / name for name in [
        "horizon-core", "horizon-world", "horizon-items", "horizon-mod-runtime", "horizon-client-core"]}
    manifest = '[package]\nname="structure-context-proof"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[lib]\npath="lib.rs"\n[dependencies]\nserde_json="1"\nlog="0.4"\n'
    manifest += 'mod_api={package="horizon-mod-api",path=' + json.dumps(str(ROOT / "crates/horizon-mod-api")) + ',features=["testing"]}\n'
    for name, path in deps.items():
        manifest += name + '={path=' + json.dumps(str(path)) + '}\n'
    profiles = (ROOT / "Cargo.toml").read_text().split("[profile.dev]", 1)[1].split("[profile.release]", 1)[0]
    (OUT / "Cargo.toml").write_text(manifest + "\n[profile.dev]" + profiles)
    shutil.copyfile(ROOT / "Cargo.lock", OUT / "Cargo.lock")

    source = (GAME / "api_values.rs").read_text()
    values = 'use horizon_items::items::Items;\n'
    values += '#[path=' + json.dumps(str(GAME / "api_values/block_state.rs")) + '] pub(crate) mod block_state;\n'
    values += between(source, "pub(in crate::game) fn resolve_block_definitions(", "impl TryIntoApi for horizon_client::boss_overlay")
    values += between(source, "impl TryIntoApi for horizon_world::chunk::BlockState", "#[cfg(test)]")
    values += between(source, "fn block_metadata_table()", "impl super::Game")
    values += between(source, "pub(crate) trait TryIntoApi {", "/// Tests use")
    values += between(source, "pub(crate) trait TryIntoEngine<", "#[derive(Clone, Copy")
    values += between(source, "fn with_rule_schema<T>", "#[cfg(test)]")
    (OUT / "api_values.rs").write_text(values)

    # Retain exact production items. Only the two test module paths change.
    adapter = (GAME / "native_structure_types.rs").read_text()
    adapter = adapter.replace('"native_structure_types/tests.rs"', json.dumps(str(OUT / "structure_tests.rs")))
    adapter = adapter.replace('"../../../../../docs/modding/examples/src/worldgen.rs"', json.dumps(str(ROOT / "docs/modding/examples/src/worldgen.rs")))
    (OUT / "structure.rs").write_text(adapter)
    (OUT / "structure_tests.rs").write_text((GAME / "native_structure_types/tests.rs").read_text() + ALLOCATION_TEST)
    modules = {"api_values": OUT / "api_values.rs", "native_structure_types": OUT / "structure.rs"}
    modules.update({name: GAME / (name + ".rs") for name in ["native_worldgen", "native_dimensions", "native_chunk_generator"]})
    body = "#![allow(dead_code)]\npub use horizon_core::json_nbt;\n" + ALLOCATOR + "\npub mod game {\n"
    for name, path in modules.items():
        body += '#[path=' + json.dumps(str(path)) + '] pub(crate) mod ' + name + ';\n'
    (OUT / "lib.rs").write_text(body + "}\n")


ALLOCATOR = r'''
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell};
thread_local! {
    static WATCH: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
struct Counted;
fn tick() {
    if WATCH.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get()+1));
    }
}
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 { tick(); unsafe { System.alloc(layout) } }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 { tick(); unsafe { System.alloc_zeroed(layout) } }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 { tick(); unsafe { System.realloc(ptr, layout, size) } }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { unsafe { System.dealloc(ptr, layout) } }
}
#[global_allocator] static ALLOCATOR: Counted = Counted;
fn counted<T>(f: impl FnOnce() -> T) -> (T, usize) {
    struct Stop;
    impl Drop for Stop { fn drop(&mut self) { WATCH.set(false); } }
    assert!(!WATCH.get());
    ALLOCATIONS.set(0); WATCH.set(true);
    let stop = Stop;
    let answer = f();
    drop(stop);
    (answer, ALLOCATIONS.get())
}
'''

ALLOCATION_TEST = r'''
#[test]
fn worker_queries_allocate_only_final_column_outputs() {
    let resources = test_resources();
    let worker = WorkerResources::get(resources.registry).unwrap();
    let mut engine = StubContext;
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    let config = Arc::new(Tag::End);
    let mut start = Start { inner:&mut engine, random:Draws(&mut random), chunk:ChunkPos::new(0,0), config:&config, worker };
    let (column, allocations) = crate::counted(|| start.noise_column(0,0));
    assert_eq!(allocations, 1, "only the final API blocks vector");
    assert_eq!(column.blocks, [NoiseBlock::Air, NoiseBlock::Water, NoiseBlock::Lava, NoiseBlock::Solid, NoiseBlock::Other]);
    use HeightmapType::*;
    let kinds = [WorldSurfaceWg,WorldSurface,OceanFloorWg,OceanFloor,MotionBlocking,MotionBlockingNoLeaves];
    let (columns, allocations) = crate::counted(|| start.base_columns(0,0,&kinds));
    assert_eq!(allocations, 7, "one outer vector and six final columns, no kinds or conversion vector");
    for (index,column) in columns.iter().enumerate() {
        assert_eq!(column, &(0..6).map(|slot|slot==index).collect::<Vec<_>>());
    }
    let (_, allocations) = crate::counted(|| {
        (start.seed(),start.sea_level(),start.min_y(),start.max_y(),start.config(),start.noise_biome([0;3]),start.random().next_long())
    });
    assert_eq!(allocations,0,"stack adapters and scalar projections");
}
'''


def check(filter_name="game::"):
    prepare()
    result = subprocess.run(["cargo", "test", "--manifest-path", str(OUT / "Cargo.toml"), "--lib", filter_name], cwd=ROOT, capture_output=True, text=True, timeout=590)
    print(result.stdout, end="", flush=True)
    print(result.stderr, end="", flush=True)
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--mutants", action="store_true")
    parser.add_argument("--api-mutant", action="store_true")
    args = parser.parse_args()
    assert check().returncode == 0, "baseline must pass"
    if args.api_mutant:
        path = ROOT / "crates/horizon-mod-api/src/world/structure_type.rs"
        original = path.read_bytes()
        old = b"i64::from(y) - i64::from(self.min)"
        assert original.count(old) == 1
        command = ["cargo", "test", "-p", "horizon-mod-api", "--features", "wasm-host", "--lib", "noise_column_clips_outside_y_without_overflow"]
        def api_check():
            result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=590)
            print(result.stdout + result.stderr, end="", flush=True)
            return result
        assert api_check().returncode == 0
        (OUT / "api-mutation-backup.rs").write_bytes(original)
        try:
            path.write_bytes(original.replace(old, b"i64::from(y - self.min)"))
            result = api_check()
            assert result.returncode == 101 and "test result: FAILED" in result.stdout
            print("KILLED: API Y subtraction overflow", flush=True)
        finally:
            path.write_bytes(original)
        assert api_check().returncode == 0
        return
    if not args.mutants:
        return
    mutants = [
        (GAME / "native_worldgen.rs", "if !std::ptr::eq(resource.metadata, metadata)", "if false && !std::ptr::eq(resource.metadata, metadata)", "racing_owners"),
        (GAME / "native_structure_types.rs", "Engine::Water => NoiseBlock::Water", "Engine::Water => NoiseBlock::Lava", "structure_column_projection"),
        (GAME / "native_structure_types.rs", "crate::json_nbt::json_to_tag(config)", "crate::json_nbt::json_to_tag(&serde_json::Value::Null)", "structure_config_is_converted"),
        (GAME / "native_dimensions.rs", "LevelHeight::of_json(&dimension_type(settings)).is_some()", "true", "dimension_shape_edges"),
    ]
    for path, old, new, name in mutants:
        original = path.read_text()
        assert original.count(old) == 1, (path, old)
        (OUT / "mutation-backup.rs").write_text(original)
        try:
            path.write_text(original.replace(old, new))
            result = check(name)
            assert result.returncode == 101 and "test result: FAILED" in result.stdout, name
            print("KILLED:", name, flush=True)
        finally:
            path.write_text(original)
    assert check().returncode == 0, "restored baseline must pass"


if __name__ == "__main__":
    main()
