import contextlib
import io
import json
import os
import shutil
import sys
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path

from tools import prebuilt_game


class RegistryPatchTests(unittest.TestCase):
    def test_sdk_carries_only_patched_crate_and_resolution_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            staged, sdk = root / "stage", root / "sdk"
            patched = staged / "third_party" / "allocator-sys"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (staged / "Cargo.toml").write_text("[workspace]\nmembers=[]\n[patch.crates-io]\nallocator-sys={path='third_party/allocator-sys'}\n")
            (patched / "Cargo.toml").write_text("[package]\nname='allocator-sys'\nversion='0.1.49'\n")
            (patched / "src").mkdir()
            (patched / "src/lib.rs").write_text("pub fn patched() {}")
            (staged / "game.rs").write_text("must not ship")
            patches = prebuilt_game.carry_patches(staged, sdk)
            self.assertEqual(patches["crates-io"]["allocator-sys"]["path"], "patches/allocator-sys")
            self.assertEqual((sdk / "patches/allocator-sys/src/lib.rs").read_text(), "pub fn patched() {}")
            self.assertFalse((sdk / "game.rs").exists())
            self.assertNotIn("workspace", (sdk / "Cargo.toml").read_text())

    def test_source_symlink_is_refused_without_copying_its_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, sdk = root / "source", root / "sdk"
            patched = source / "patch"
            patched.mkdir(parents=True)
            sdk.mkdir()
            (source / "Cargo.toml").write_text("[patch.crates-io]\nallocator-sys={path='patch'}\n")
            outside = root / "private.txt"
            outside.write_text("must not ship")
            (patched / "leak").symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "symlink"):
                prebuilt_game.carry_patches(source, sdk)
            self.assertFalse((sdk / "patches").exists())

    def test_output_inventory_rejects_extra_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            patch = sdk / "patches/allocator-sys"
            patch.mkdir(parents=True)
            declared = patch / "Cargo.toml"
            declared.write_text("manifest")
            (sdk / "deps").mkdir()
            for crate in ("horizon_mod_wasm", "horizon_mod_host"):
                (sdk / "deps" / f"lib{crate}.so").write_bytes(b"")
            (sdk / prebuilt_game.CATALOG).write_text("{}")
            (sdk / prebuilt_game.IDENTITY).write_text(json.dumps({"patch_files": {
                "patches/allocator-sys/Cargo.toml": prebuilt_game.sha256(declared)}, "files": {
                prebuilt_game.CATALOG: prebuilt_game.sha256(sdk / prebuilt_game.CATALOG)}}))
            prebuilt_game.check(sdk)
            extra = patch / "extra.rs"
            extra.write_text("not inventoried")
            with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
                prebuilt_game.check(sdk)
            extra.unlink()
            extra.symlink_to(declared)
            with self.assertRaisesRegex(SystemExit, "symlinks"):
                prebuilt_game.check(sdk)
            extra.unlink()
            declared.write_text("modified")
            with self.assertRaisesRegex(SystemExit, "inventory mismatch"):
                prebuilt_game.check(sdk)


class DeterministicArchiveTests(unittest.TestCase):
    def test_workflow_archive_is_independent_of_creation_order_and_mtimes(self):
        # Execute the workflow command itself, so a regression in its flags breaks this proof.
        workflow = (prebuilt_game.ROOT / ".github/workflows/prebuilt-game.yml").read_text()
        command = next(line.strip() for line in workflow.splitlines() if "tar --sort=name" in line)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archives = []
            for index, order in enumerate([("a", "b"), ("b", "a")]):
                work = root / str(index)
                payload = work / "prebuilt/patches/crate"
                payload.mkdir(parents=True)
                for name in order:
                    file = payload / name
                    file.write_text(name)
                    os.utime(file, (1000 + index, 1000 + index))
                for folder in [payload, payload.parent, payload.parent.parent]:
                    os.utime(folder, (2000 + index, 2000 + index))
                subprocess.run(["bash", "-o", "pipefail", "-c", command], cwd=work,
                               env={**os.environ, "ARCHIVE": "sdk.tar.gz"}, check=True)
                archives.append((work / "sdk.tar.gz").read_bytes())
            self.assertEqual(archives[0], archives[1])


class AllocatorSelectionTests(unittest.TestCase):
    def test_staging_api_version_comes_from_the_package_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "crates/horizon-mod-package/src/manifest.rs"
            manifest.parent.mkdir(parents=True)
            manifest.write_text('pub const API: &str = "7.3";\n')
            self.assertEqual(prebuilt_game.api_version(root), "7.3")
            manifest.write_text('pub const OTHER: &str = "7.3";\n')
            with self.assertRaisesRegex(ValueError, "manifest API"):
                prebuilt_game.api_version(root)

    def test_windows_archives_use_the_system_allocator(self):
        self.assertEqual(prebuilt_game.cargo_features(False, "win32"), ["--features", "system-allocator"])
        self.assertEqual(prebuilt_game.cargo_features(True, "win32"),
                         ["--no-default-features", "--features", "system-allocator"])

    def test_other_systems_keep_mimalloc(self):
        for platform in ("linux", "darwin"):
            self.assertEqual(prebuilt_game.cargo_features(False, platform), [])
            self.assertEqual(prebuilt_game.cargo_features(True, platform), ["--no-default-features"])

    def test_build_uses_explicit_host_target_and_target_dynamic_flags(self):
        class Captured(Exception):
            pass

        def capture(command, **kwargs):
            self.assertEqual(command[command.index("--target") + 1], "fixture-host")
            self.assertIn("-C prefer-dynamic", kwargs["env"]["RUSTFLAGS"])
            self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", kwargs["env"])
            raise Captured

        with patch.object(prebuilt_game, "stage", return_value=Path("staged")), \
             patch.object(prebuilt_game, "rustc_host", return_value="fixture-host"), \
             patch.object(prebuilt_game, "run", side_effect=capture), \
             patch.dict(prebuilt_game.os.environ, {"CARGO_ENCODED_RUSTFLAGS": "-Cprefer-dynamic=no"}):
            with self.assertRaises(Captured):
                prebuilt_game.build(Path("unused-sdk"), False)


class ExecutableLinkPolicyTests(unittest.TestCase):
    def test_windows_sdk_identity_keeps_the_source_executable_stack_policy(self):
        compiler = shutil.which("rustc")
        if compiler is None:
            self.skipTest("requires Rust to build and run the engine's build script")
        target = prebuilt_game.ROOT / "target"
        target.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=target) as directory:
            root = Path(directory)
            # Isolate unrelated code generation/resource compilation, but execute the actual
            # engine build script and its real shared link policy, as the SDK build does.
            stubs = {
                "horizon_game_discovery": 'pub fn declarations(_: &std::path::Path, _: &[&str]) '
                    '-> std::io::Result<String> { Ok(String::new()) }',
                "horizon_mod_api_gen": 'pub enum Target { Client } '
                    'pub fn generate(_: &std::path::Path, _: &std::path::Path, _: Target) '
                    '-> std::io::Result<()> { Ok(()) }',
                "winresource": 'pub struct WindowsResource; impl WindowsResource { '
                    'pub fn new() -> Self { Self } pub fn set_icon(&mut self, _: &str) {} '
                    'pub fn set_manifest_file(&mut self, _: &str) {} '
                    'pub fn compile(&self) -> Result<(), String> { Ok(()) } }',
            }
            externs = []
            for name, code in stubs.items():
                source = root / f"{name}.rs"
                source.write_text(code)
                library = root / f"lib{name}.rlib"
                subprocess.run([compiler, "--edition=2024", "--crate-type=rlib", str(source),
                                "-o", str(library)], check=True)
                externs.extend(["--extern", f"{name}={library}"])
            binary = root / ("producer.exe" if sys.platform == "win32" else "producer")
            subprocess.run([compiler, "--edition=2024",
                            str(prebuilt_game.ROOT / "crates/horizon-client/build.rs"),
                            *externs, "-o", str(binary)], check=True)
            for target_os, env, expected in [("windows", "msvc", ["/STACK:8388608"]),
                                      ("windows", "gnu", ["-Wl,--stack,8388608"]),
                                      ("linux", "gnu", []), ("macos", "", [])]:
                with self.subTest(os=target_os, env=env):
                    output = root / "output"
                    output.write_text(subprocess.check_output([str(binary)], text=True, env={
                        **os.environ,
                        "CARGO_MANIFEST_DIR": str(prebuilt_game.ROOT / "crates/horizon-client"),
                        "OUT_DIR": str(root), "CARGO_PKG_NAME": "horizon-client",
                        "CARGO_CFG_TARGET_OS": target_os, "CARGO_CFG_TARGET_ENV": env,
                    }))
                    identity = {"link_args": prebuilt_game.executable_link_args(output)}
                    self.assertEqual(identity["link_args"], expected)


class PrebuiltContentsTests(unittest.TestCase):
    def folder(self, *names: str) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for name in names:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_bytes(b"")
        return root

    def test_only_target_artifacts_and_host_proc_macros_enter_the_archive(self):
        root = self.folder(
            "cache/release/deps/libshared-host.rmeta",
            "cache/release/deps/libderive.so",
            "cache/host/release/deps/libshared-target.rmeta",
            "cache/host/release/deps/libshared-second-version.rmeta",
            "cache/host/release/deps/libshared-target.rlib",
            "cache/host/release/deps/libhorizon_client.so",
        )
        output, macros = root / "sdk/deps", root / "sdk/host"
        output.mkdir(parents=True)
        macros.mkdir()
        def artifact(path, kind):
            return {"reason": "compiler-artifact", "target": {"kind": [kind]},
                    "filenames": [str(root / path)]}
        prebuilt_game.copy_artifacts([
            artifact("cache/release/deps/libshared-host.rmeta", "lib"),
            artifact("cache/release/deps/libderive.so", "proc-macro"),
            artifact("cache/host/release/deps/libshared-target.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-second-version.rmeta", "lib"),
            artifact("cache/host/release/deps/libshared-target.rlib", "lib"),
            artifact("cache/host/release/deps/libhorizon_client.so", "dylib"),
            {"reason": "build-finished", "success": True},
        ], output, macros, root / "cache/host")
        self.assertEqual({path.name for path in output.iterdir()}, {
            "libshared-target.rmeta", "libshared-second-version.rmeta", "libhorizon_client.so",
        })
        self.assertEqual({path.name for path in macros.iterdir()}, {"libderive.so"})
        # Multiple legitimate target versions remain available to rustc's transitive lookup.
        # Only explicit extern/facade lookup must reject ambiguous names.
        with self.assertRaises(ValueError):
            prebuilt_game.extern_args(output, "shared")

    def test_facades_exclude_build_only_and_ambiguous_runtime_crates(self):
        def package(name, kind="lib"):
            return {"id": name, "name": name, "targets": [{"name": name.replace('-', '_'), "kind": [kind]}]}

        def node(name, *dependencies):
            return {"id": name, "deps": [
                {"pkg": target, "dep_kinds": [{"kind": kind}]} for target, kind in dependencies
            ]}

        metadata = {
            "packages": [package("horizon-mod-host", "dylib"), package("horizon-client"),
                         package("horizon-mod-wasm", "rlib"), package("point_shape"), package("horizon_core"),
                         package("build_tool"), package("derive", "proc-macro"), package("macro_helper")],
            "resolve": {"nodes": [
                node("horizon-mod-host", ("horizon-client", None), ("horizon-mod-wasm", None),
                     ("build_tool", "build")), node("horizon-mod-wasm"),
                node("horizon-client", ("point_shape", None), ("horizon_core", None), ("derive", None)),
                node("point_shape"), node("horizon_core"), node("build_tool"),
                node("derive", ("macro_helper", None)), node("macro_helper"),
            ]},
        }
        runtime = prebuilt_game.runtime_crates(metadata, "horizon-mod-host")
        self.assertEqual(runtime, {"horizon_mod_host", "horizon_mod_wasm", "horizon_client", "point_shape", "horizon_core"})
        deps = self.folder("libpoint_shape-host.rmeta", "libpoint_shape-target.rmeta",
                           "libhorizon_core-one.rmeta", "libbuild_tool-one.rmeta", "libmacro_helper-one.rmeta",
                           "libhorizon_mod_host.so", "libhorizon_mod_wasm-one.rmeta", "libhorizon_client-one.rmeta")
        eligible = prebuilt_game.facade_crates(deps, runtime)
        self.assertEqual(eligible, ["horizon_client", "horizon_core", "horizon_mod_host", "horizon_mod_wasm"])

    def test_native_link_inventory_uses_package_identity_not_a_same_named_host_variant(self):
        def package(package_id, version, library=None):
            return {"id": package_id, "name": "native-sys" if library else "horizon-mod-host",
                    "version": version, "links": library,
                    "targets": [{"name": "native_sys" if library else "horizon_client", "kind": ["lib"]}]}
        metadata = {
            "packages": [package("game", "1.0.0"), package("target", "1.0.0", "native"),
                         package("host", "2.0.0", "native")],
            "resolve": {"nodes": [
                {"id": "game", "deps": [{"pkg": "target", "dep_kinds": [{"kind": None}]},
                                          {"pkg": "host", "dep_kinds": [{"kind": "build"}]}]},
                {"id": "target", "deps": []}, {"id": "host", "deps": []},
            ]},
        }
        self.assertEqual(prebuilt_game.native_links(metadata, "horizon-mod-host"), [
            {"name": "native-sys", "version": "1.0.0", "library": "native"},
        ])

    def test_sdk_assertion_refuses_a_duplicate_eligible_facade_on_every_os(self):
        root = self.folder("deps/libhorizon_core-one.rmeta", "deps/libmod_api-one.rmeta",
                           "deps/libhorizon_mod_package-one.rmeta")
        prebuilt_game.assert_externs(root, ["horizon_core"])
        (root / "deps/libhorizon_core-two.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for horizon_core"):
            prebuilt_game.assert_externs(root, ["horizon_core"])

    def test_native_extern_lookup_refuses_a_second_package_variant(self):
        root = self.folder("libhorizon_mod_package-runtime.rmeta")
        self.assertEqual(prebuilt_game.extern_args(root, "horizon_mod_package"), [
            "--extern", f"horizon_mod_package={root / 'libhorizon_mod_package-runtime.rmeta'}",
        ])
        (root / "libhorizon_mod_package-build.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(ValueError, "expected exactly one SDK metadata for horizon_mod_package"):
            prebuilt_game.extern_args(root, "horizon_mod_package")

    def test_host_macros_pass_but_host_dependency_metadata_is_refused(self):
        root = self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", "deps/libhorizon_mod_host.so",
                           "host/libderive.so", "host/derive.dll", "host/libderive.dylib")
        (root / prebuilt_game.CATALOG).write_bytes(b"{}")
        (root / prebuilt_game.IDENTITY).write_text(json.dumps({"files": {
            prebuilt_game.CATALOG: prebuilt_game.sha256(root / prebuilt_game.CATALOG)}}))
        prebuilt_game.check(root)
        (root / "host/libshared-host.rmeta").write_bytes(b"")
        with self.assertRaisesRegex(SystemExit, "aren't compiler outputs"):
            prebuilt_game.check(root)

    def test_unrelated_multiversion_dependencies_do_not_break_named_externs(self):
        root = self.folder("deps/libmod_api-one.rmeta", "deps/libhorizon_mod_package-one.rmeta",
                           "deps/libshared-v1.rmeta", "deps/libshared-v2.rmeta",
                           "host/libmod_api-host.so")
        prebuilt_game.assert_externs(root, [])

    def test_compiler_outputs_pass(self):
        root = self.folder(
            "horizon-prebuilt.json", "Cargo.lock", "native_main.rs.template", "deps/libhorizon_client.so", "deps/libmod_api-1.rmeta",
            "native/libwindows.0.52.0.a", "points.json", "internals.json", "mod-catalog.json",
            "deps/libserde_derive-2.so", "deps/libhorizon_client.dll.a", "deps/libhorizon_mod_wasm.so",
            "deps/horizon_mod_wasm.dll", "deps/libhorizon_mod_wasm.dylib", "deps/libhorizon_mod_host.so",
        )
        (root / prebuilt_game.IDENTITY).write_text(json.dumps({"files": {
            prebuilt_game.CATALOG: prebuilt_game.sha256(root / prebuilt_game.CATALOG)}}))
        prebuilt_game.check(root)
        (root / prebuilt_game.CATALOG).write_text("changed")
        with self.assertRaisesRegex(SystemExit, "catalog"):
            prebuilt_game.check(root)
        (root / prebuilt_game.CATALOG).unlink()
        with self.assertRaisesRegex(SystemExit, "catalog"):
            prebuilt_game.check(root)

    def test_anything_else_is_refused(self):
        for stray in ("assets/minecraft/lang/en_us.json", "deps/Cargo.lock", "deps/client.jar", "deps/libmod_api-3.rlib", "native/libfoo.so", "deps/internals.json", "notes.txt"):
            with self.subTest(stray), self.assertRaises(SystemExit):
                prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", stray))

    def test_a_folder_without_either_runtime_is_refused(self):
        for present, missing in (("horizon_mod_host", "horizon-mod-wasm"), ("horizon_mod_wasm", "horizon-mod-host")):
            with self.subTest(present), self.assertRaisesRegex(SystemExit, missing):
                prebuilt_game.check(self.folder(f"deps/lib{present}.so"))

    def test_host_carries_engine_code_with_engine_metadata_for_mods(self):
        root = self.folder("deps/libhorizon_mod_host.so", "deps/libhorizon_mod_wasm.so", "deps/libhorizon_client-123.rmeta", "mod-catalog.json")
        (root / prebuilt_game.IDENTITY).write_text(json.dumps({"files": {
            prebuilt_game.CATALOG: prebuilt_game.sha256(root / prebuilt_game.CATALOG)}}))
        prebuilt_game.check(root)

    def test_engine_and_wasm_without_host_are_refused(self):
        with self.assertRaisesRegex(SystemExit, "horizon-mod-host"):
            prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so"))

    def dll(self, root: Path, name: str, count: int, pe64: bool = True) -> None:
        # One PE section: RVA 0x1000 maps to raw offset 0x200, holding the export directory.
        data = bytearray(0x400)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<H", data, 0x86, 1)  # NumberOfSections
        optional_size = 240 if pe64 else 224
        struct.pack_into("<H", data, 0x94, optional_size)
        struct.pack_into("<H", data, 0x98, 0x20B if pe64 else 0x10B)
        struct.pack_into("<II", data, 0x98 + (112 if pe64 else 96), 0x1000, 40)
        struct.pack_into("<III", data, 0x98 + optional_size + 12, 0x1000, 0x200, 0x200)
        struct.pack_into("<I", data, 0x200 + 24, count)  # NumberOfNames
        (root / "deps" / name).write_bytes(data)

    def test_exports_reports_every_dll(self):
        root = self.folder("deps/libhorizon_client.so")
        self.dll(root, "horizon_client.dll", 45000)
        self.dll(root, "horizon_mod_wasm.dll", 12000)
        self.dll(root, "serde_derive.dll", 1, pe64=False)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(root)
        for name, count in (("horizon_client", 45000), ("horizon_mod_wasm", 12000), ("serde_derive", 1)):
            self.assertIn(f"{name}.dll: {count} exported names", output.getvalue())

    def test_exports_warns_about_any_dll_near_the_limit_without_failing(self):
        for large in ("horizon_client", "horizon_mod_wasm"):
            root = self.folder("deps/libhorizon_client.so")
            for name in ("horizon_client", "horizon_mod_wasm"):
                self.dll(root, f"{name}.dll", 49000 if name == large else 48999)
            output = io.StringIO()
            with self.subTest(large), contextlib.redirect_stdout(output):
                prebuilt_game.exports(root)
            warnings = [line for line in output.getvalue().splitlines() if line.startswith("::warning::")]
            self.assertEqual(len(warnings), 1)
            self.assertIn(f"{large}.dll exports 49,000 names", warnings[0])
            self.assertEqual(output.getvalue().count("exported names (Windows' limit"), 2)

    def test_exports_is_silent_without_dlls(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            prebuilt_game.exports(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so"))
        self.assertEqual(output.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
