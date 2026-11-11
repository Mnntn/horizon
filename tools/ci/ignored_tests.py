#!/usr/bin/env python3
"""Run every library test, ignored or not, with the prepared vanilla data, against a list of known failures.

Usage: ignored_tests.py SHARD...  (HORIZON_TEST_RESOURCES points at the prepared 26.3 data)

A SHARD is a package, or one leg of it written `PACKAGE#NofM` (tools/ci/ignored_shards.py splits the biggest
package so one process does not hold every test's memory). A leg runs the tests whose name hashes to its
number, so every test runs in exactly one leg, and only judges the list's entries for those same tests.
An unqualified horizon-client-lib runs two such selections consecutively on the same runner, releasing
retained fixture memory between processes without adding a matrix job.

Each package's library tests run with `--include-ignored`: the #[ignore]d ones that need the data, and the
ones that return early without it, which `verify` (no data) passes without checking anything. The packages
in INTEGRATION_TESTS also run their `tests/` integration tests. The job fails when a test fails that
tools/ci/ignored-tests-known-failures.txt doesn't list, when a listed test passes (take it off the
list: the list only shrinks), and when a listed test no longer exists. A test marked `flaky` there
may pass or fail; one marked `harness` is a parity harness's step, run by its harness with the
environment it sets, and is skipped here; one marked `gpu` needs a graphics adapter and runs only where
the runner has a Vulkan driver (an ICD file), and must pass there.

Runtime diagnostics expose identities and allowlisted panic categories, never arbitrary assertion
payloads: the prepared data is Mojang's. Compile/list failures expose bounded compiler headings.
"""

import os
import io
from functools import lru_cache
import re
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import lab

KNOWN = Path(__file__).with_name("ignored-tests-known-failures.txt")
RESULT = re.compile(r"^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED)$")
SUMMARY = re.compile(r"^test result: \S+\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;")
TARGET = re.compile(r"^\s+Running (?:unittests|tests/)")
COUNT = re.compile(r"^running (\d+) tests?$")
PANIC = re.compile(r"^thread '([^']+)'.* panicked at (\S+):$")
# Packages whose integration tests (`tests/*.rs`) run too, beside their library tests.
INTEGRATION_TESTS = {"horizon-client"}
LEG = re.compile(r"^(?P<package>[^#]+)#(?P<number>\d+)of(?P<count>\d+)$")
LISTED = re.compile(r"^(\S+): test$")
ICD_DIRS = ("/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d")


def compile_diagnostics(output):
    """List mode never executes tests. Show headings, not source excerpts or arbitrary Cargo output."""
    headings = []
    for line in output.splitlines():
        if re.match(r"^error(?:\[E\d+\])?:", line):
            # Compiler/proc-macro diagnostics can contain resource-derived text even
            # without quotes. Only fixed reason forms cross this boundary.
            code = re.match(r"^error(\[E\d+\])?:", line)[1] or ""
            reason = line.split(":", 1)[1].strip()
            safe = reason if reason in {
                "mismatched types", "type annotations needed", "unexpected closing delimiter",
                "unclosed delimiter", "expected item, found keyword", "aborting due to previous error",
            } else "custom diagnostic payload withheld"
            if reason.startswith("linking with "):
                safe = "linker failure; command/output withheld"
                if status := re.search(r"(?:^|: )exit status: (\d{1,5})(?:\D|$)", reason):
                    safe += f"; exit status={status[1]}"
            elif reason.startswith("failed to run custom build command for "):
                safe = "custom build command failure; command/output withheld"
            headings.append(f"error{code}: {safe}")
        elif re.match(r"^\s*--> [\w./-]+:\d+:\d+$", line):
            headings.append(line)
    for line in headings[:20]:
        print("  compiler: " + line[:300] + (" [shortened]" if len(line) > 300 else ""))
        if line.strip().startswith("--> "):
            panic_source(line.strip()[4:], kind="compiler")
    if len(headings) > 20:
        print(f"  compiler diagnostics truncated: {len(headings) - 20} more headings")
    if not headings:
        print("  no recognized compiler diagnostic; non-diagnostic output withheld")


@lru_cache(maxsize=32)
def source_lines(path):
    """Only immutable tracked Rust blobs, never a working-tree or resource file."""
    if not re.fullmatch(r"(?:crates|launcher)/[A-Za-z0-9_./-]+\.rs", path):
        return None
    if any(part in ("", ".", "..", "target", ".local") for part in path.split("/")):
        return None
    # ls-tree mode rejects symlinks and bounds blobs before git-show allocates output.
    entry = subprocess.run(["git", "ls-tree", "-l", "HEAD", "--", path],
                           capture_output=True, text=True, errors="replace")
    fields = entry.stdout.split()
    if entry.returncode or len(fields) != 5 or fields[0] not in ("100644", "100755") or fields[1] != "blob":
        return None
    if not fields[3].isdigit() or int(fields[3]) > 256_000:
        return None
    blob = subprocess.run(["git", "show", f"HEAD:{path}"], capture_output=True, text=True, errors="replace")
    return blob.stdout.splitlines() if blob.returncode == 0 else None


def panic_source(location, kind="panic"):
    found = re.fullmatch(r"(.+\.rs):(\d+):(\d+)", location)
    lines = source_lines(found[1]) if found else None
    if lines is None or not 1 <= int(found[2]) <= len(lines):
        print(f"  {kind} source unavailable (not a bounded tracked Rust location)")
        return
    line = lines[int(found[2]) - 1]
    # Prefix and escape controls so checked-in text cannot inject workflow commands.
    line = line.encode("unicode_escape").decode("ascii")
    print(f"  {kind} source: " + line[:300] + (" [shortened]" if len(line) > 300 else ""))


def has_vulkan():
    """Whether this machine has a Vulkan driver for wgpu to find."""
    return bool(os.environ.get("VK_ICD_FILENAMES")) or any(
        Path(d).is_dir() and any(Path(d).glob("*.json")) for d in ICD_DIRS
    )


def parse_shard(shard):
    """(package, leg number or None, leg count or None) of a shard argument."""
    found = LEG.match(shard)
    if not found:
        return shard, None, None
    number, count = int(found["number"]), int(found["count"])
    if not 1 <= number <= count:
        sys.exit(f"{shard}: leg {number} of {count} doesn't exist")
    return found["package"], number, count


def in_leg(test, number, count):
    """Whether `test` belongs to leg `number` of `count` (every test to exactly one leg; all, with no legs)."""
    return number is None or zlib.crc32(test.encode()) % count == number - 1


def known_failures():
    """(package, test) -> its mark (None, "flaky", "harness" or "gpu"), from the checked-in list."""
    known = {}
    for number, line in enumerate(KNOWN.read_text().splitlines(), 1):
        line = line.split(" # ", 1)[0].split()
        if not line or line[0].startswith("#"):
            continue
        if len(line) < 3 or line[3:] not in ([], ["flaky"], ["harness"], ["gpu"]):
            sys.exit(f"{KNOWN.name}:{number}: expected `PACKAGE TEST REFERENCE [flaky|harness|gpu]`")
        known[(line[0], line[1])] = line[3] if len(line) > 3 else None
    return known


# Budgets include UTF-8 newlines; summary metadata is outside the payload budget.
FAILURE_BYTES = 500
FAILURE_TOTAL_BYTES = 128_000
DIAGNOSTIC_LINES = 100_000
DIAGNOSTIC_LINE_CHARS = 4096
DIAGNOSTIC_CONTEXTS = 4096
_active_budget = None


def safe_identity(name):
    # Rust path segments only, not arbitrary libtest payloads or GitHub tokens.
    if re.fullmatch(r"(?:r#)?[^\W\d]\w*(?:::(?:r#)?[^\W\d]\w*)*", name) and not re.search(
            r"(?:gh[pousr]_|github_pat_|token|secret)", name, re.IGNORECASE):
        return name
    return "identity withheld (unclassified test name)"


@lru_cache(maxsize=1)
def tracked_rust_paths():
    listing = subprocess.run(["git", "ls-tree", "-rl", "HEAD", "--", "crates", "launcher"],
                             capture_output=True, text=True, errors="replace")
    paths = set()
    if listing.returncode == 0:
        for line in listing.stdout.splitlines():
            fields = line.split()
            if (len(fields) == 5 and fields[0] in ("100644", "100755") and
                    fields[1] == "blob" and fields[3].isdigit() and int(fields[3]) <= 256_000 and
                    re.fullmatch(r"(?:crates|launcher)/[A-Za-z0-9_./-]+\.rs", fields[4]) and
                    not any(part in ("", ".", "..", "target", ".local") for part in fields[4].split("/"))):
                paths.add(fields[4])
    return paths


def bounded_record(record, remaining):
    raw = record.encode("utf-8")
    shortened = len(raw) > FAILURE_BYTES
    if shortened:
        marker = b" [shortened: per-test byte budget]\n"
        raw = raw[:max(0, FAILURE_BYTES - len(marker))].decode("utf-8", errors="ignore").encode("utf-8") + marker
    if len(raw) > remaining[0]:
        return False, False
    print(raw.decode("utf-8"), end="")
    remaining[0] -= len(raw)
    return True, shortened


def failure_diagnostics(output):
    """One record per failed result occurrence, including repeated names across targets.

    Panic contexts use libtest's name model and the first panic for that name.
    They cannot disambiguate identical names in different targets.
    """
    failures = []
    contexts = {}
    context = []
    panic_reason_pending = False
    skipped_lines = 0
    skipped_contexts = 0
    stream = io.StringIO(output)
    for index, line in enumerate(stream):
        if index >= DIAGNOSTIC_LINES:
            skipped_lines += 1 + sum(1 for _ in stream)
            break
        if len(line) > DIAGNOSTIC_LINE_CHARS:
            skipped_lines += 1
            panic_reason_pending = False
            continue
        line = line.rstrip("\r\n")
        if RESULT.match(line) or SUMMARY.match(line) or TARGET.match(line):
            panic_reason_pending = False
        if panic_reason_pending and line.strip():
            context[:] = [item for item in context if item != "  panic reason missing from captured output"]
            # No free-form panic text or assertion operands cross the resource boundary.
            reason = next((label for prefix, label in (
                ("assertion `left == right` failed", "assertion equality failed"),
                ("assertion `left != right` failed", "assertion inequality failed"),
                ("assertion failed:", "assertion condition failed"),
                ("called `Result::unwrap()`", "Result::unwrap on an error"),
                ("called `Option::unwrap()`", "Option::unwrap on None"),
                ("index out of bounds", "index out of bounds"),
                ("attempt to", "arithmetic failure"),
            ) if line.strip().startswith(prefix)), "custom panic")
            if arithmetic := re.fullmatch(r"attempt to ((?:add|subtract|multiply|divide|negate|shift left|shift right) with overflow|(?:divide|calculate the remainder) by zero)", line.strip()):
                reason = "attempt to " + arithmetic[1]
            context.append(f"  panic reason: {reason}; payload withheld (resource privacy)")
            panic_reason_pending = False
            continue
        if found := RESULT.match(line):
            if found[2] == "FAILED":
                failures.append(found[1])
        if found := PANIC.match(line):
            context = []
            if found[1] not in contexts:
                if len(contexts) < DIAGNOSTIC_CONTEXTS:
                    contexts[found[1]] = context
                else:
                    skipped_contexts += 1
                    continue
            location = re.fullmatch(r"(.+\.rs):(\d+):(\d+)", found[2])
            path = location[1] if location else ""
            if any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in path):
                path = ""
            # game-discovery emits canonical absolute #[path] declarations. Only
            # this checkout's exact resolved prefix is authoritative; never guess
            # another checkout's prefix or search for a matching path suffix.
            prefix = str(Path(__file__).resolve().parents[2]) + os.sep
            if path.startswith(prefix):
                path = path[len(prefix):]
            # Rust cross-crate source paths can contain lexical ../ segments.
            # Never resolve an untrusted path against the filesystem, and never
            # allow a traversal to leave the root, even if it later returns.
            parts = []
            for part in path.split("/"):
                if part == "..":
                    if not parts:
                        parts = []
                        break
                    parts.pop()
                elif part not in ("", "."):
                    parts.append(part)
            path = "/".join(parts) if not path.startswith("/") else ""
            lines = source_lines(path) if path in tracked_rust_paths() else None
            if (location and lines is not None and
                    1 <= int(location[2]) <= len(lines) and int(location[3]) >= 1):
                context.append(f"  panicked at {path}:{location[2]}:{location[3]}")
            else:
                context.append("  panic location withheld (not a bounded tracked Rust location)")
            context.append("  panic reason missing from captured output")
            panic_reason_pending = True
    remaining = _active_budget if _active_budget is not None else [FAILURE_TOTAL_BYTES]
    shown = shortened = missing = withheld = 0
    identities_withheld = locations_withheld = payloads_withheld = reasons_shown = 0
    for name in failures:
        identity = safe_identity(name)
        details = contexts.get(name, ["  panic context missing from captured output"])
        missing += any("missing from captured output" in item for item in details)
        withheld += identity != name or any("withheld" in item for item in details)
        identities_withheld += identity != name
        locations_withheld += any("panic location withheld" in item for item in details)
        payloads_withheld += any("payload withheld" in item for item in details)
        reasons_shown += any(item.startswith("  panic reason:") for item in details)
        record = f"  failed test: {identity}\n" + "\n".join(details) + "\n"
        emitted, cut = bounded_record(record, remaining)
        shown += emitted
        shortened += cut
    print(f"failure diagnostics: {shown} shown; {len(failures) - shown} failing tests not shown "
          f"(total byte budget {FAILURE_TOTAL_BYTES}); {shortened} records shortened "
          f"(per-test byte budget {FAILURE_BYTES}); {missing} missing context; {withheld} withheld context/identity; "
          f"{reasons_shown} classified panic reasons; {identities_withheld} identities withheld; "
          f"{locations_withheld} locations withheld; {payloads_withheld} payloads withheld (expected privacy filtering); "
          f"{skipped_lines} lines skipped (line/count bounds; failure count unknown there); "
          f"{skipped_contexts} contexts skipped (retention bound)")


def run(package, skip, number=None, count=None):
    """Each test's result in `package`'s ignored library tests but `skip` (and, with a leg, the other legs'), or None if they didn't build."""
    # Resource-backed fixtures need a fresh home, not the runner user's game files.
    with tempfile.TemporaryDirectory(prefix="horizon-ignored-") as scratch:
        homes = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
            home = Path(scratch) / key.lower()
            home.mkdir()
            homes[key] = home
        for key in ("HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            homes[key] = Path(scratch) / key.lower()
        env = lab.game_env(**homes)
        if package == "horizon-client-lib":
            # Worldgen must meet its real-time deadlines without disabling test assertions.
            env["CARGO_PROFILE_TEST_OPT_LEVEL"] = "3"
        # --no-fail-fast: a target with accepted known failures must not stop the targets after it.
        # Owner-approved stopgap; #2944 removes this when the legs run under 15 minutes.
        client_half = package == "horizon-client" and count == 2 and number in (1, 2)
        command = ["cargo", "test", "-p", package, "--lib", *(["--bins", "--tests"] if package in INTEGRATION_TESTS else []), "--no-fail-fast"]
        if client_half:
            command.append("--jobs=8")
        expected = set()
        timing = os.environ.get("HORIZON_DATA_LEG_TIMING") == "1"
        phase = f"{package}#{number}of{count}" if number is not None else package
        if timing:
            print(f"data-leg phase={phase} compile-list-start utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}", flush=True)
        compile_started = time.monotonic()
        if number is not None:
            # List what exists, then skip every test another leg owns.
            listing = subprocess.run(
                [*command, "--", "--list", "--include-ignored"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace", env=env,
            )
            names = {found[1] for line in listing.stdout.splitlines() if (found := LISTED.match(line))}
            if listing.returncode or not names:
                print(f"{package}: could not list the tests to split into legs (cargo exit {listing.returncode})")
                compile_diagnostics(listing.stderr)
                return None
            others = sorted(name for name in names if not in_leg(name, number, count))
            # Every test this leg owns and does not skip must report a result.
            expected = set(names) - set(others) - set(skip)
            skip = [*skip, *others]
            print(f"{package}: leg {number} of {count} runs {len(names) - len(others)} of {len(names)} listed tests")
        skips = [arg for test in skip for arg in ("--skip", test)]
        if timing:
            # Listing already compiled the target(s). Do not forward its output.
            # This marks the existing test command, not an instruction inside a binary.
            print(f"data-leg phase={phase} test-command-start utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} listed={int(number is not None)} compile_list_seconds={time.monotonic() - compile_started:.3f}", flush=True)
        execution_started = time.monotonic()
        threads = 8 if client_half else 4
        process = subprocess.run(
            [*command, "--", "--include-ignored", "--exact", f"--test-threads={threads}", *skips],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", env=env,
        )
        if timing:
            print(f"data-leg phase={phase} test-command-end utc={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} execution_seconds={time.monotonic() - execution_started:.3f}", flush=True)
    results = {}
    summaries = []
    print(f"{package}: cargo exit {process.returncode}")
    target_count = None
    target_number = 0
    awaiting_count = False
    incomplete = False
    failure_diagnostics(process.stdout)
    for line in process.stdout.splitlines():
        if TARGET.match(line):
            incomplete |= target_count is not None or awaiting_count
            target_number += 1
            awaiting_count = True
        elif awaiting_count and (found := COUNT.match(line)):
            target_count = int(found[1])
            print(f"{package}: target {target_number} started {target_count} tests")
            awaiting_count = False
        if found := RESULT.match(line):
            results[found[1]] = found[2] == "ok"
        elif line.startswith("screen-effect event loop error: "):
            print(line)
        elif found := SUMMARY.match(line):
            # Resource fixtures launch filtered copies of this test executable.
            # Their summaries are not evidence that the parent target finished.
            if target_count == sum(map(int, found.groups())):
                summaries.append(line)
                target_count = None
                print(f"{package}: {line}")
        elif status := re.search(r"\((?:signal|exit status): \d+", line):
            # Emit only the numeric status, never adjacent test output.
            print(f"{package}: {status[0]})")
    if incomplete or awaiting_count or target_count is not None:
        print(f"{package}: test target did not finish (cargo exit {process.returncode})")
        return None
    if process.returncode and not target_number:
        compile_diagnostics(process.stdout)
    if process.returncode and all(results.values()):
        print(f"{package}: cargo exited {process.returncode} without a parsed test failure")
        return None
    if missing := sorted(expected - results.keys()):
        print(f"{package}: {len(missing)} listed tests of this leg never reported a result:")
        remaining = _active_budget if _active_budget is not None else [FAILURE_TOTAL_BYTES]
        shown = 0
        for name in missing:
            emitted, _ = bounded_record(f"  {safe_identity(name)}\n", remaining)
            shown += emitted
        print(f"{len(missing) - shown} missing identities not shown (shared total byte budget)")
        return None
    if not summaries:
        # Unknown output is not necessarily a compiler diagnostic: never forward
        # a malformed test process's captured payload to public CI logs.
        print(f"{package}: no completed test-target summary; raw output withheld")
        return None
    return results


def main(shards):
    global _active_budget
    _active_budget = [FAILURE_TOTAL_BYTES]
    known = known_failures()
    skipped = {"harness"} if has_vulkan() else {"harness", "gpu"}
    if "gpu" in skipped:
        print("no Vulkan driver on this runner: the tests marked `gpu` are skipped")
    problems = []
    accepted_unshown = 0
    # Reuse the hash selection in two consecutive processes on the same runner.
    # Resource-backed client-lib families retain registries across tests; releasing
    # the process between halves bounds that retention without another CI leg.
    selections = [
        selection
        for shard in shards
        for selection in (
            ("horizon-client-lib#1of2", "horizon-client-lib#2of2")
            if shard == "horizon-client-lib" else (shard,)
        )
    ]
    for shard in selections:
        package, number, count = parse_shard(shard)
        skip = [test for (listed, test), mark in known.items() if listed == package and mark in skipped]
        results = run(package, skip, number, count) if number is not None else run(package, skip)
        if results is None:
            problems.append(f"{shard}: the test run did not complete")
            continue
        for test, passed in sorted(results.items()):
            listed = (package, test) in known
            policy_test = test
            test = safe_identity(test)
            if not passed and listed and known[(package, policy_test)] != "gpu":
                emitted, _ = bounded_record(
                    f"{package} {test}: accepted failure policy={KNOWN.name} "
                    f"mark={known[(package, policy_test)] or 'known'}\n", _active_budget)
                accepted_unshown += not emitted
            if not passed and not listed:
                problems.append(f"{package} {test}: failed, and isn't a known failure")
            elif not passed and known[(package, policy_test)] == "gpu":
                problems.append(f"{package} {test}: failed with a Vulkan driver present")
            elif passed and listed and known[(package, policy_test)] is None:
                problems.append(f"{package} {test}: passes now; take it off {KNOWN.name}")
        for (listed, test), mark in known.items():
            if listed == package and mark not in skipped and in_leg(test, number, count) and test not in results:
                problems.append(f"{package} {safe_identity(test)}: listed in {KNOWN.name} but no longer runs")
    shown = 0
    for problem in problems:
        emitted, _ = bounded_record(f"::error::{problem}\n", _active_budget)
        shown += emitted
    if problems:
        print(f"policy diagnostics: {len(problems) - shown} problems not shown (shared total byte budget)")
    if accepted_unshown:
        print(f"{accepted_unshown} accepted policy records not shown (shared total byte budget)")
    _active_budget = None
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
