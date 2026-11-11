#!/usr/bin/env python3
"""Move the mod vocabulary source files verbatim (#2534).

Run from the repository root. --verify compares moved files with the source
commit recorded below, independently of Git's rename similarity heuristic.
Boundary adaptations belong in later commits, not in this script.
"""
import argparse
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
OLD = pathlib.Path('crates/horizon-mod-api/src')
NEW = pathlib.Path('crates/horizon-mod-vocab/src')
BASE = 'e96c76c0b9'
STAY = {
    'wire/wire_guest.rs', 'wire/wire_resources.rs',
    'world/phase.rs', 'world/reload.rs', 'world/server_reload.rs',
    'client/client_reload.rs', 'content/registry_keys.rs',
    'testing.rs', 'parts/shared.rs',
}


def files(base=BASE):
    names = subprocess.check_output(
        ['git', 'ls-tree', '-r', '--name-only', base, str(OLD)], cwd=ROOT,
        text=True,
    ).splitlines()
    return [pathlib.Path(name).relative_to(OLD) for name in names
            if pathlib.Path(name).relative_to(OLD).as_posix() not in STAY
            and not pathlib.Path(name).relative_to(OLD).as_posix().startswith('testing/')]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify', action='store_true')
    parser.add_argument('--at', help='Verify the verbatim move commit, before boundary fixes')
    parser.add_argument('--closure', action='store_true', help='Move the dispatch dependency closure')
    parser.add_argument('--source-base', default=BASE, help='Source tree for incremental moves')
    parser.add_argument('--new-since', help='Select every source file added since this tree')
    parser.add_argument('--audit', action='store_true', help='Reject vocabulary files left in the facade')
    args = parser.parse_args()
    if args.audit:
        remaining = [p.relative_to(ROOT / OLD).as_posix()
                     for p in (ROOT / OLD).rglob('*') if p.is_file()]
        unexpected = set(remaining) - {'lib.rs', 'wire/wire_guest.rs'}
        if unexpected:
            raise SystemExit(f'vocabulary left in facade: {sorted(unexpected)}')
        print('facade holds only lib.rs and wire/wire_guest.rs')
        return
    paths = files(args.source_base)
    if args.new_since:
        previous = set(subprocess.check_output(
            ['git', 'ls-tree', '-r', '--name-only', args.new_since, str(OLD)],
            cwd=ROOT, text=True).splitlines())
        paths = [p for p in paths if str(OLD / p) not in previous]
        paths = [p for p in paths if p.as_posix() != 'lib.rs']
    if args.closure:
        names = subprocess.check_output([
            "git", "ls-tree", "-r", "--name-only", args.source_base, str(OLD)
        ], cwd=ROOT, text=True).splitlines()
        paths = [pathlib.Path(name).relative_to(OLD) for name in names
                 if pathlib.Path(name).relative_to(OLD) not in paths
                 and not name.endswith("/wire/wire_guest.rs")]
    for path in paths:
        destination = ROOT / NEW / path
        if args.verify:
            expected = subprocess.check_output(['git', 'show', f'{args.source_base}:{OLD / path}'], cwd=ROOT)
            actual = (subprocess.check_output(["git", "show", f"{args.at}:{NEW / path}"], cwd=ROOT)
                      if args.at else destination.read_bytes())
            if actual != expected:
                raise SystemExit(f'non-verbatim move: {path}')
        else:
            if not (ROOT / OLD / path).exists() and destination.exists():
                # Git's directory-rename merge already carried this added file.
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run(['git', 'mv', str(OLD / path), str(NEW / path)], cwd=ROOT, check=True)
    print(f'{len(paths)} verbatim moves {"verified" if args.verify else "performed"}')


if __name__ == '__main__':
    main()
