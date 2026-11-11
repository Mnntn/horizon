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

Only test names, results, panic locations and explicitly marked screen-effect event loop errors
are printed: the prepared data is Mojang's, and a failing assertion could quote it.
"""

import os
import re
import subprocess
import sys
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import lab
from tools.ci.frozen_scratch import owned_scratch

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


def run(package, skip, number=None, count=None):
    """Each test's result in `package`'s ignored library tests but `skip` (and, with a leg, the other legs'), or None if they didn't build."""
    # Resource-backed fixtures need a fresh home, not the runner user's game files.
    with owned_scratch() as (scratch, scratch_env):
        homes = {}
        for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
            home = Path(scratch) / key.lower()
            home.mkdir()
            homes[key] = home
        for key in ("HORIZON_LAUNCHER_TRASH", "HORIZON_LAUNCHER_CLIPBOARD", "HORIZON_LAUNCHER_DESKTOP"):
            homes[key] = Path(scratch) / key.lower()
        env = lab.game_env(**homes)
        env.update(scratch_env)
        if package == "horizon-client-lib":
            # Worldgen must meet its real-time deadlines without disabling test assertions.
            env["CARGO_PROFILE_TEST_OPT_LEVEL"] = "3"
        # --no-fail-fast: a target with accepted known failures must not stop the targets after it.
        command = ["cargo", "test", "-p", package, "--lib", *(["--bins", "--tests"] if package in INTEGRATION_TESTS else []), "--no-fail-fast"]
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
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, errors="replace", env=env,
            )
            names = {found[1] for line in listing.stdout.splitlines() if (found := LISTED.match(line))}
            if listing.returncode or not names:
                print(f"{package}: could not list the tests to split into legs (cargo exit {listing.returncode})")
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
        process = subprocess.run(
            [*command, "--", "--include-ignored", "--exact", "--test-threads=4", *skips],
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
        elif found := PANIC.match(line):
            print(f"  {found[1]} panicked at {found[2]}")
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
    if process.returncode and all(results.values()):
        print(f"{package}: cargo exited {process.returncode} without a parsed test failure")
        return None
    if missing := sorted(expected - results.keys()):
        print(f"{package}: {len(missing)} listed tests of this leg never reported a result:")
        for name in missing[:50]:
            print(f"  {name}")
        return None
    if not summaries:
        # Unknown output is not necessarily a compiler diagnostic: never forward
        # a malformed test process's captured payload to public CI logs.
        print(f"{package}: no completed test-target summary; raw output withheld")
        return None
    return results


def main(shards):
    known = known_failures()
    skipped = {"harness"} if has_vulkan() else {"harness", "gpu"}
    if "gpu" in skipped:
        print("no Vulkan driver on this runner: the tests marked `gpu` are skipped")
    problems = []
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
            if not passed and not listed:
                problems.append(f"{package} {test}: failed, and isn't a known failure")
            elif not passed and known[(package, test)] == "gpu":
                problems.append(f"{package} {test}: failed with a Vulkan driver present")
            elif passed and listed and known[(package, test)] is None:
                problems.append(f"{package} {test}: passes now; take it off {KNOWN.name}")
        for (listed, test), mark in known.items():
            if listed == package and mark not in skipped and in_leg(test, number, count) and test not in results:
                problems.append(f"{package} {test}: listed in {KNOWN.name} but no longer runs")
    for problem in problems:
        print(f"::error::{problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
