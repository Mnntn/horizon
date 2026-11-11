#!/usr/bin/env python3
"""The prebuilt game (#589): horizon-client as a Rust dylib plus every crate's metadata, so the launcher builds
native trait mods against it with rustc alone, in seconds, without the game's source or a game build.

    python3 tools/prebuilt_game.py build OUT [--server]   # build and collect the folder, write horizon-prebuilt.json
    python3 tools/prebuilt_game.py smoke OUT              # build the starter mod and the game executable against OUT only
    python3 tools/prebuilt_game.py exports OUT            # each DLL's exported names, toward Windows' limit

Each OS's folder is built on that OS: the proc-macro libraries in it run inside the player's own rustc.
The folder holds compiler outputs and game-owned registry patch crates, never vanilla data; `build` refuses any other file.
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROFILE = "release"
# What rustc writes for a dylib or proc macro on each OS, plus metadata and the MinGW import libraries.
SUFFIXES = (".rmeta", ".so", ".dll", ".dll.a", ".dylib")
IDENTITY = "horizon-prebuilt.json"
LOCK = "Cargo.lock"
# The launcher's generated `main.rs` comes from the game it links against.
TEMPLATE = "native_main.rs.template"
# Source files the folder carries besides compiler outputs.
CARRIED = {"Cargo.toml": None, IDENTITY: None, LOCK: ROOT / LOCK, TEMPLATE: ROOT / "crates/horizon-client/src" / TEMPLATE}
# What the game's build script asks the `horizon-client` executable's link for (Windows' 8 MB main stack).
LINK_ARG = "cargo:horizon-executable-link-arg="
# Native libraries the build scripts point the final link at (the `windows` crates' MinGW import libraries):
# cargo passes their folders to rustc, a plain rustc gets them from here (`-L native=OUT/native`).
NATIVE = "native"
NATIVE_SUFFIXES = (".a", ".lib")
# The game's injection points, listed by the staging pass: the launcher routes mods' point injections by it.
POINTS = "points.json"
CATALOG = "mod-catalog.json"
# The staged internals signatures, checked against a mod's declared access list.
INTERNALS = "internals.json"


def run(command: list[str], **kwargs) -> str:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, **kwargs).stdout


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def rustc_host() -> str:
    verbose = run(["rustc", "-vV"])
    return next(line.split(": ", 1)[1] for line in verbose.splitlines() if line.startswith("host: "))


def api_version(root: Path) -> str:
    """Read the owning manifest's API constant without a generator/package dependency."""
    source = (root / "crates/horizon-mod-package/src/manifest.rs").read_text()
    matches = re.findall(r'^pub const API: &str = ("[^"\n]*");$', source, re.MULTILINE)
    if len(matches) != 1:
        raise ValueError("expected one manifest API version")
    return json.loads(matches[0])


def stage(target_dir: Path) -> Path:
    """A copy of the tracked source with the staging pass run over it (#589): every point method gets its
    `#[point]`, internals become public, and both manifests are written. The working tree is never rewritten."""
    staged = target_dir / "stage"
    if staged.exists():
        shutil.rmtree(staged)
    tracked = run(["git", "ls-files", "-z"], cwd=ROOT).split("\0")
    for name in filter(None, tracked):
        source = ROOT / name
        if source.is_file():
            (staged / name).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, staged / name)
    print(run(["cargo", "run", "--locked", "--release", "-q", "-p", "horizon-mod-api-gen", "--bin", "horizon-stage",
               "--", str(staged), str(staged / POINTS), str(staged / INTERNALS), api_version(staged)],
              cwd=staged, env={**os.environ, "CARGO_TARGET_DIR": str(target_dir / "tools")}), end="")
    # Only the prebuilt game splits the wasm runtime out: normal source builds stay static. With
    # prefer-dynamic, rustc picks this dylib for the game rather than folding its runtime stack into it.
    for crate in ("horizon-mod-wasm",):
        with (staged / f"crates/{crate}/Cargo.toml").open("a") as manifest:
            manifest.write('\n[lib]\ncrate-type = ["rlib", "dylib"]\n')
    return staged


def cargo_features(server: bool, platform: str) -> list[str]:
    """The game crate's features for this archive. On Windows the game keeps no allocator of its own (#2394):
    std is a DLL there with its own default-allocator shim, and Windows binds each import to the DLL that exports
    it, so a mimalloc `#[global_allocator]` in the game would leave the mods' executable and std's own code on
    the system heap beside it. Elsewhere the first definition wins and every library shares the game's mimalloc."""
    features = ["--no-default-features"] if server else []
    if platform == "win32":
        features += ["--features", "system-allocator"]
    return features


def copy_artifacts(messages: list[dict], deps: Path, host: Path, target_dir: Path) -> None:
    """Keep target metadata separate from the host proc-macros loaded by rustc.

    An explicit Cargo --target keeps host build dependencies outside target_dir;
    those libraries are not part of the target SDK. Proc-macros alone run on the host.
    """
    for message in messages:
        if message["reason"] != "compiler-artifact":
            continue
        proc_macro = "proc-macro" in message["target"]["kind"]
        for name in message["filenames"]:
            file = Path(name)
            if not name.endswith(SUFFIXES):
                continue
            if proc_macro:
                if name.endswith((".so", ".dll", ".dylib", ".dll.a")):
                    shutil.copy2(file, host / file.name)
            elif file.is_relative_to(target_dir):
                shutil.copy2(file, deps / file.name)


def runtime_packages(metadata: dict, root: str) -> set[str]:
    """Normal SDK library package IDs, excluding host build/proc-macro trees."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [package["id"] for package in metadata["packages"] if package["name"] == root]
    visited = set()
    runtime = set()
    while pending:
        package_id = pending.pop()
        if package_id in visited:
            continue
        visited.add(package_id)
        if not any(any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])
                   for target in packages[package_id]["targets"]):
            continue
        runtime.add(package_id)
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return runtime


def runtime_crates(metadata: dict, root: str) -> set[str]:
    runtime = runtime_packages(metadata, root)
    return {target["name"] for package in metadata["packages"] if package["id"] in runtime
            for target in package["targets"]
            if any(kind in ("lib", "rlib", "dylib") for kind in target["kind"])}


def native_links(metadata: dict, root: str) -> list[dict]:
    runtime = runtime_packages(metadata, root)
    return [{"name": package["name"], "version": package["version"], "library": package["links"]}
            for package in metadata["packages"] if package["id"] in runtime and package.get("links")]


def facade_crates(deps: Path, runtime: set[str]) -> list[str]:
    """Only runtime crates with a unique extern can be exposed by an internals facade."""
    eligible = []
    for crate in sorted(runtime):
        if dylib(deps, crate) is not None or len(list(deps.glob(f"lib{crate}-*.rmeta"))) == 1:
            eligible.append(crate)
    return eligible


def assert_externs(out: Path, eligible: list[str]) -> None:
    """Runs in the SDK build on every OS, including macOS, before publication."""
    for crate in set(eligible) | {"mod_api", "horizon_mod_package"}:
        extern_args(out / "deps", crate)


def build(out: Path, server: bool) -> None:
    # Resolved here: the staging and build steps run from the staged copy.
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "prebuilt"
    staged = stage(target_dir)
    host = rustc_host()
    # One std for the game, every mod and the executable: the game links std dynamically, as mods will.
    rustflags = "-C prefer-dynamic"
    # On macOS this also gives both dylibs an @rpath install name instead of an absolute build path.
    if sys.platform == "darwin":
        rustflags += " -C rpath"
    features = cargo_features(server, sys.platform)
    # Linux's executable RUNPATH doesn't cover transitive dependencies: the game itself must find
    # the wasm runtime beside it. macOS's @rpath install names use the executable's rpath.
    link = ["--", "-C", "link-arg=-Wl,-rpath,$ORIGIN"] if sys.platform == "linux" else []
    # Encoded flags take precedence over RUSTFLAGS in Cargo.
    environment = {key: value for key, value in os.environ.items() if key != "CARGO_ENCODED_RUSTFLAGS"}
    messages = run(
        ["cargo", "rustc", "--locked", "-p", "horizon-mod-host", "--lib", "--crate-type", "dylib",
         "--target", host, "--profile", PROFILE, "--message-format", "json-render-diagnostics", *features, *link],
        cwd=staged,
        env={**environment, "RUSTFLAGS": rustflags + " -D warnings", "CARGO_TARGET_DIR": str(target_dir)},
    )
    artifacts = list(map(json.loads, messages.splitlines()))
    scripts = [
        message for message in artifacts
        if message["reason"] == "build-script-executed"
    ]
    if out.exists():
        shutil.rmtree(out)
    deps = out / "deps"
    deps.mkdir(parents=True)
    macros = out / "host"
    macros.mkdir()
    copy_artifacts(artifacts, deps, macros, target_dir / host)
    metadata = json.loads(run([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host,
        *features,
    ], cwd=staged))
    eligible = facade_crates(deps, runtime_crates(metadata, "horizon-mod-host"))
    assert_externs(out, eligible)
    native = out / NATIVE
    native.mkdir()
    # Only libraries that come with a crate (its source in Cargo's home, or its build script's output): a
    # system folder a script names (pkg-config's /usr/lib) is the player's own system's to provide.
    crates = (target_dir, Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    for script in scripts:
        # A static library a script names is already inside its crate's rlib, so inside the game.
        bundled = {lib.split("=")[-1] for lib in script["linked_libs"] if lib.startswith("static")}
        for search in script["linked_paths"]:
            folder = Path(search.split("=", 1)[-1]).resolve()
            if not any(folder.is_relative_to(root.resolve()) for root in crates):
                continue
            for file in sorted(folder.iterdir()):
                stem = file.name.removeprefix("lib").split(".")[0]
                if file.is_file() and file.name.endswith(NATIVE_SUFFIXES) and stem not in bundled:
                    shutil.copy2(file, native / file.name)
    for name, source in CARRIED.items():
        if source is not None:
            shutil.copy2(source, out / name)
    patches = carry_patches(staged, out)
    game = next(script for script in scripts if script["package_id"].split("#")[0].endswith("/horizon-client"))
    for manifest in (POINTS, INTERNALS, CATALOG):
        shutil.copy2(staged / manifest, out / manifest)
    output = Path(game["out_dir"]).parent / "output"
    link_args = executable_link_args(output)
    files = {str(path.relative_to(out).as_posix()): sha256(path) for path in sorted(out.rglob("*")) if path.is_file()}
    identity = {
        "format_version": 5,
        "commit": run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip(),
        "rustc": run(["rustc", "-vV"]).strip(),
        "target": host,
        "profile": PROFILE,
        "side": "server" if server else "client",
        "rustflags": rustflags,
        "link_args": link_args,
        "facade_crates": eligible,
        "registry_patches": patches,
        "patch_files": {name: digest for name, digest in files.items() if name.startswith("patches/")},
        "registry_lock": (staged / "Cargo.lock").read_text(),
        "native_links": native_links(metadata, "horizon-mod-host"),
        "files": files,
    }
    (out / IDENTITY).write_text(json.dumps(identity, indent=2) + "\n")
    check(out)


def carry_patches(staged: Path, out: Path) -> dict:
    """Carry only game-owned registry patch crates, never the rest of the game source."""
    patches = tomllib.loads((staged / "Cargo.toml").read_text()).get("patch", {})
    for registry in patches.values():
        for spec in registry.values():
            source = staged / spec["path"]
            if source.is_symlink() or any(path.is_symlink() for path in source.rglob("*")):
                raise ValueError("game patch contains a symlink")
            source = source.resolve()
            if not source.is_relative_to(staged.resolve()):
                raise ValueError("game patch escapes the staged source")
            destination = out / "patches" / source.name
            shutil.copytree(source, destination, symlinks=True)
            if any(path.is_symlink() for path in destination.rglob("*")):
                raise ValueError("copied game patch contains a symlink")
            spec["path"] = str(destination.relative_to(out))
    # This deliberately is not the game's buildable workspace manifest.
    lines = []
    for registry, entries in patches.items():
        lines.append(f"[patch.{json.dumps(registry)}]")
        for name, spec in entries.items():
            lines.append(f"{json.dumps(name)} = {{ path = {json.dumps(spec['path'])} }}")
    (out / "Cargo.toml").write_text("\n".join(lines) + "\n")
    return patches


def executable_link_args(output: Path) -> list[str]:
    """Executable policy emitted by the engine library's build script into the SDK identity."""
    return [line[len(LINK_ARG):] for line in output.read_text().splitlines() if line.startswith(LINK_ARG)]


def dylib(deps: Path, crate: str) -> Path | None:
    """A dylib carries its own metadata, so it has no `.rmeta` beside it."""
    names = {f"{prefix}{crate}.{ext}" for prefix in ("lib", "") for ext in ("so", "dll", "dylib")}
    return next((path for path in deps.iterdir() if path.name in names), None)


def extern_args(deps: Path, crate: str) -> list[str]:
    """Resolve a native SDK extern by name, refusing ambiguous metadata rather than choosing a variant."""
    library = dylib(deps, crate)
    if library is None:
        candidates = list(deps.glob(f"lib{crate}-*.rmeta"))
        if len(candidates) != 1:
            raise ValueError(f"expected exactly one SDK metadata for {crate}: {candidates}")
        library = candidates[0]
    return ["--extern", f"{crate}={library}"]


def check(out: Path) -> None:
    """Only compiler outputs and exactly inventoried patch sources may ship."""
    identity = json.loads((out / IDENTITY).read_text()) if (out / IDENTITY).is_file() else {}
    patch_files = identity.get("patch_files", {})
    links = [path for path in out.rglob("*") if path.is_symlink()]
    if links:
        sys.exit(f"prebuilt folder contains symlinks: {links}")
    for name, digest in patch_files.items():
        path = out / name
        if not name.startswith("patches/") or ".." in Path(name).parts or not path.is_file() or sha256(path) != digest:
            sys.exit(f"prebuilt patch inventory mismatch: {name}")
    stray = [
        path for path in out.rglob("*")
        if path.is_file() and not (path.parent == out and (path.name in CARRIED or path.name in (POINTS, INTERNALS, CATALOG)))
        and str(path.relative_to(out).as_posix()) not in patch_files
        and not (path.parent == out / "deps" and path.name.endswith(SUFFIXES))
        and not (path.parent == out / "host" and path.name.endswith((".so", ".dll", ".dylib", ".dll.a")))
        and not (path.parent == out / NATIVE and path.name.endswith(NATIVE_SUFFIXES))
    ]
    if stray:
        sys.exit(f"prebuilt folder holds files that aren't compiler outputs: {stray}")
    for crate in ("horizon_mod_wasm", "horizon_mod_host"):
        if dylib(out / "deps", crate) is None:
            sys.exit(f"prebuilt folder has no {crate.replace('_', '-')} dylib")

    identity = json.loads((out / IDENTITY).read_text())
    expected = identity.get("files", {}).get(CATALOG)
    if not expected or not (out / CATALOG).is_file() or sha256(out / CATALOG) != expected:
        sys.exit("prebuilt catalog is missing or does not match its identity")


def smoke(out: Path) -> None:
    """Build what a player's launcher builds: the starter mod and the generated executable, from OUT alone."""
    deps = out / "deps"
    assert_externs(out, json.loads((out / IDENTITY).read_text())["facade_crates"])

    def extern(crate: str) -> list[str]:
        return extern_args(deps, crate)

    common = ["--edition", "2024", "-C", "prefer-dynamic", "-C", "opt-level=1", "-L", f"dependency={deps}",
              "-L", f"dependency={out / 'host'}", "-L", f"native={out / NATIVE}"]
    with tempfile.TemporaryDirectory(dir=out.parent) as scratch:
        scratch = Path(scratch)
        starter = (ROOT / "crates/cargo-horizon-mod/template/native/lib.rs").read_text()
        (scratch / "mod_0.rs").write_text(starter.replace("{id}", "smoke"))
        run(["rustc", *common, "--crate-type", "rlib", "--crate-name", "mod_0", *extern("mod_api"),
             "--out-dir", str(scratch), str(scratch / "mod_0.rs")])
        template = (out / TEMPLATE).read_text()
        main = template.replace("/* MOD_ENTRIES */", '        "smoke" => mod_0::hooks,\n').replace(
            "/* BUILT_MODS */",
            '    horizon_mod_package::runtime::BuiltMod { id: "smoke", version: "0.1.0", side: horizon_mod_package::Side::Both },\n',
        )
        (scratch / "main.rs").write_text(main)
        link_args = json.loads((out / IDENTITY).read_text())["link_args"]
        run(["rustc", *common, "--crate-type", "bin", "--crate-name", "horizon_client_smoke",
             *(flag for arg in link_args for flag in ("-C", f"link-arg={arg}")),
             *extern("horizon_client"), *extern("mod_api"), *extern("horizon_mod_package"), "--extern", f"horizon_mod_host={dylib(deps, 'horizon_mod_host')}",
             "--extern", f"mod_0={scratch / 'libmod_0.rlib'}", "-L", f"dependency={scratch}",
             "-o", str(scratch / "horizon-client"), str(scratch / "main.rs")])
    print(f"smoke: the starter mod and the game executable built against {out} alone")


# Warns here, ahead of Windows' hard 65,535 (the linker's own failure), so a growing export list is seen first.
EXPORT_WARNING = 49_000


def export_count(library: Path) -> int:
    """Windows' exported-name count from a DLL's PE export directory, readable on any OS."""
    data = library.read_bytes()
    pe = int.from_bytes(data[0x3C:0x40], "little")
    optional = pe + 24
    directories = optional + (112 if int.from_bytes(data[optional:optional + 2], "little") == 0x20B else 96)
    export_rva = int.from_bytes(data[directories:directories + 4], "little")
    sections = optional + int.from_bytes(data[pe + 20:pe + 22], "little")
    for i in range(int.from_bytes(data[pe + 6:pe + 8], "little")):
        at = sections + 40 * i
        address, raw_size, raw = (int.from_bytes(data[at + o:at + o + 4], "little") for o in (12, 16, 20))
        if address <= export_rva < address + raw_size:
            directory = raw + export_rva - address
            names = int.from_bytes(data[directory + 24:directory + 28], "little")
            return names
    sys.exit(f"{library.name} has no export directory")


def exports(out: Path) -> None:
    """Report every DLL's Windows export count (#589), the separate wasm runtime included, and warn on
    one near the limit: the linker's own failure at 65,535 is the real one."""
    for library in sorted((out / "deps").glob("*.dll")):
        names = export_count(library)
        print(f"{library.name}: {names} exported names (Windows' limit is 65,535)")
        if names >= EXPORT_WARNING:
            warning = (f"{library.name} exports {names:,} names, {65_535 - names:,} short of Windows' 65,535: "
                       "cut exported names before adding points or internals (#589)")
            print(f"::warning::{warning}")
            if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
                with open(summary, "a", encoding="utf-8") as file:
                    file.write(f"- {warning}\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build")
    build_parser.add_argument("out", type=Path)
    build_parser.add_argument("--server", action="store_true", help="the dedicated server's features")
    smoke_parser = commands.add_parser("smoke")
    smoke_parser.add_argument("out", type=Path)
    exports_parser = commands.add_parser("exports")
    exports_parser.add_argument("out", type=Path)
    args = parser.parse_args()
    if args.command == "build":
        build(args.out.resolve(), args.server)
    elif args.command == "exports":
        exports(args.out.resolve())
    else:
        smoke(args.out.resolve())


if __name__ == "__main__":
    main()
