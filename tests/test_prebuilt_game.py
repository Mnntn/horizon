import contextlib
import io
import struct
import tempfile
import unittest
from pathlib import Path

from tools import prebuilt_game


class AllocatorSelectionTests(unittest.TestCase):
    def test_windows_archives_use_the_system_allocator(self):
        self.assertEqual(prebuilt_game.cargo_features(False, "win32"), ["--features", "system-allocator"])
        self.assertEqual(prebuilt_game.cargo_features(True, "win32"),
                         ["--no-default-features", "--features", "system-allocator"])

    def test_other_systems_keep_mimalloc(self):
        for platform in ("linux", "darwin"):
            self.assertEqual(prebuilt_game.cargo_features(False, platform), [])
            self.assertEqual(prebuilt_game.cargo_features(True, platform), ["--no-default-features"])


class PrebuiltContentsTests(unittest.TestCase):
    def folder(self, *names: str) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        for name in names:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_bytes(b"")
        return root

    def test_only_current_cargo_artifacts_enter_the_archive(self):
        root = self.folder(
            "cache/libpoint_shape-old.rmeta", "cache/libpoint_shape-host.rmeta",
            "cache/libpoint_shape-target.rmeta", "cache/libpoint_shape-target.rlib",
            "cache/libderive.so", "cache/libhorizon_client.so",
        )
        output = root / "sdk"
        output.mkdir()
        current = ["libpoint_shape-host.rmeta", "libpoint_shape-target.rmeta",
                   "libpoint_shape-target.rlib", "libderive.so", "libhorizon_client.so"]
        prebuilt_game.copy_artifacts([
            {"reason": "compiler-artifact", "filenames": [str(root / "cache" / name) for name in current]},
            {"reason": "build-finished", "success": True},
        ], output)
        self.assertEqual({path.name for path in output.iterdir()}, {
            "libpoint_shape-host.rmeta", "libpoint_shape-target.rmeta", "libderive.so", "libhorizon_client.so",
        })

    def test_facades_exclude_build_only_and_ambiguous_runtime_crates(self):
        def package(name, kind="lib"):
            return {"id": name, "name": name, "targets": [{"name": name.replace('-', '_'), "kind": [kind]}]}

        def node(name, *dependencies):
            return {"id": name, "deps": [
                {"pkg": target, "dep_kinds": [{"kind": kind}]} for target, kind in dependencies
            ]}

        metadata = {
            "packages": [package("horizon-client"), package("point_shape"), package("horizon_core"),
                         package("build_tool"), package("derive", "proc-macro"), package("macro_helper")],
            "resolve": {"nodes": [
                node("horizon-client", ("point_shape", None), ("horizon_core", None),
                     ("build_tool", "build"), ("derive", None)),
                node("point_shape"), node("horizon_core"), node("build_tool"),
                node("derive", ("macro_helper", None)), node("macro_helper"),
            ]},
        }
        runtime = prebuilt_game.runtime_crates(metadata)
        self.assertEqual(runtime, {"horizon_client", "point_shape", "horizon_core"})
        deps = self.folder("libpoint_shape-host.rmeta", "libpoint_shape-target.rmeta",
                           "libhorizon_core-one.rmeta", "libbuild_tool-one.rmeta", "libmacro_helper-one.rmeta")
        eligible = prebuilt_game.facade_crates(deps, runtime)
        self.assertEqual(eligible, ["horizon_core"])

    def test_native_link_inventory_uses_package_identity_not_a_same_named_host_variant(self):
        def package(package_id, version, library=None):
            return {"id": package_id, "name": "native-sys" if library else "horizon-client",
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
        self.assertEqual(prebuilt_game.native_links(metadata), [
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

    def test_compiler_outputs_pass(self):
        prebuilt_game.check(self.folder(
            "horizon-prebuilt.json", "Cargo.lock", "native_main.rs.template", "deps/libhorizon_client.so", "deps/libmod_api-1.rmeta",
            "native/libwindows.0.52.0.a", "points.json", "internals.json",
            "deps/libserde_derive-2.so", "deps/libhorizon_client.dll.a", "deps/libhorizon_mod_wasm.so",
            "deps/horizon_mod_wasm.dll", "deps/libhorizon_mod_wasm.dylib",
        ))

    def test_anything_else_is_refused(self):
        for stray in ("assets/minecraft/lang/en_us.json", "deps/Cargo.lock", "deps/client.jar", "deps/libmod_api-3.rlib", "native/libfoo.so", "deps/internals.json", "notes.txt"):
            with self.subTest(stray), self.assertRaises(SystemExit):
                prebuilt_game.check(self.folder("deps/libhorizon_client.so", "deps/libhorizon_mod_wasm.so", stray))

    def test_a_folder_without_either_runtime_is_refused(self):
        for present, missing in (("horizon_client", "horizon-mod-wasm"), ("horizon_mod_wasm", "horizon-client")):
            with self.subTest(present), self.assertRaisesRegex(SystemExit, missing):
                prebuilt_game.check(self.folder(f"deps/lib{present}.so"))

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
