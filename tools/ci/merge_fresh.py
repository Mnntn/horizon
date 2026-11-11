#!/usr/bin/env python3
"""Read-only, advisory freshness check. Exit 0 FRESH, 1 STALE, 2 unavailable."""
import argparse
import io
import json
import re
import subprocess
import zipfile


class Unavailable(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Unavailable(message)


def sha(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-f]{40}", value), "invalid source SHA")
    return value


class GitHub:
    def __init__(self, repo):
        require(re.fullmatch(r"[\w.-]+/[\w.-]+", repo), "invalid repository")
        self.prefix = f"repos/{repo}/"

    def raw(self, endpoint):
        # No method override, mutation flags, git fetch, or checkout.
        return subprocess.run(["gh", "api", self.prefix + endpoint], check=True, capture_output=True).stdout

    def get(self, endpoint):
        return json.loads(self.raw(endpoint))

    def pages(self, endpoint, key):
        result = []
        for page in range(1, 1001):
            data = self.get(f"{endpoint}{'&' if '?' in endpoint else '?'}per_page=100&page={page}")
            rows = data[key] if key else data
            require(isinstance(rows, list), "missing paginated rows")
            result.extend(rows)
            if len(rows) < 100:
                if key and 'total_count' in data:
                    require(len(result) == data['total_count'], "incomplete paginated response")
                return result
        raise Unavailable("pagination limit reached")

    def source(self, artifact):
        raw = self.raw(f"actions/artifacts/{artifact['id']}/zip")
        require(len(raw) <= 1024 * 1024, "oversized source archive")
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            require(archive.namelist() == ['gate-source.json'], "unexpected source archive members")
            require(archive.getinfo('gate-source.json').file_size <= 16384, "oversized source record")
            return json.loads(archive.read('gate-source.json'))


def paths(rows):
    require(isinstance(rows, list), "missing files")
    result = set()
    for row in rows:
        require(isinstance(row, dict) and isinstance(row.get('filename'), str), "invalid file evidence")
        result.add(row['filename'])
        if row.get('status') == 'renamed':
            require(isinstance(row.get('previous_filename'), str), "missing rename source")
            result.add(row['previous_filename'])
    return result


def shared(path):
    return ('slots' in path.split('/') or path == 'Cargo.lock'
            or path in {'tools/fixtures/wire_enum_layouts.json',
                        'tools/fixtures/wire_enum_baseline_identities.json'}
            or bool(re.search(r'(^|/)(?:fieldless_enum_goldens|wire_enum_goldens|wire_layout_pins)\.rs$', path)))


def crate(path):
    parts = path.split('/')
    if len(parts) >= 3 and parts[0] == 'crates':
        return '/'.join(parts[:2])
    return 'launcher' if path.startswith('launcher/') else None


def module_files(label, files):
    groups = {}
    for path in sorted(files):
        directory, _, name = path.rpartition('/')
        groups.setdefault(directory + '/' if directory else './', []).append(name)
    lines = [f'{label} ({len(files)} changed paths):']
    for directory, names in sorted(groups.items()):
        lines.append(f'  {directory} ({len(names)})')
        lines.extend(f'    {name}' for name in names)
    return '\n'.join(lines)


def conflict(main_files, pr_files):
    contracts = sorted(p for p in main_files | pr_files if shared(p))
    overlap = sorted(path for path in main_files & pr_files
                     if path.startswith(('tools/ci/', '.github/workflows/')))
    common = {crate(p) for p in main_files} & {crate(p) for p in pr_files} - {None}
    if contracts:
        reason = 'shared contract changed: ' + contracts[0]
    elif overlap:
        reason = 'same file changed: ' + overlap[0]
    elif common:
        reason = 'same crate changed: ' + sorted(common)[0]
    else:
        return None
    if common:
        # Include all changed paths, not just the shared crate, for proximity judgment.
        reason += '\n' + module_files('Intervening main', main_files)
        reason += '\n' + module_files('PR', pr_files)
    return reason


def provenance(record, run, number, head, source_attempt):
    require(isinstance(record, dict), 'missing source record')
    for key, expected in [('schema_version', 1), ('pr', number), ('run_id', run['id']),
                          ('attempt', source_attempt)]:
        require(type(record.get(key)) is int and record[key] == expected, 'source mismatch: ' + key)
    for key in ('base', 'head', 'merge'):
        sha(record.get(key))
    require(record['head'] == head, 'source head mismatch')
    return record


def check(api, number):
    pr = api.get(f'pulls/{number}')
    require(pr['state'] == 'open' and pr['base']['ref'] == 'main', 'requires open PR targeting main')
    require(pr['head']['repo']['full_name'] == pr['base']['repo']['full_name'],
            'fork PR source evidence is unavailable')
    head = sha(pr['head']['sha'])
    main = sha(api.get('git/ref/heads/main')['object']['sha'])
    # GitHub caps filtered workflow searches at 1,000 results: constrain by head
    # before paginating instead of searching this repository's entire PR history.
    runs = api.pages(f'actions/workflows/dyna-ci.yml/runs?event=pull_request&head_sha={head}', 'workflow_runs')
    candidates = [r for r in runs if r.get('event') == 'pull_request' and r.get('head_sha') == head
                  and any(p.get('number') == number for p in r.get('pull_requests', []))]
    require(candidates, 'no current-head PR gate run (main/dispatch cannot substitute)')
    run = max(candidates, key=lambda r: r['id'])
    require(type(run.get('run_attempt')) is int and run['run_attempt'] > 0, 'missing run attempt')
    attempt = run['run_attempt']
    # GitHub's latest filter includes reused jobs from partial reruns, but never
    # searches older successes when the latest same-name execution is red.
    jobs = api.pages(f"actions/runs/{run['id']}/jobs?filter=latest", 'jobs')
    for name in ('lint', 'verify', 'launcher', 'pool-watch'):
        gates = [j for j in jobs if j.get('name') == name]
        require(len(gates) == 1, 'missing or ambiguous gate job: ' + name)
        allowed = {'success', 'skipped'} if name in ('launcher', 'pool-watch') else {'success'}
        require(gates[0].get('status') == 'completed' and gates[0].get('conclusion') in allowed,
                'gate pending or unsuccessful: ' + name)
    artifacts = api.pages(f"actions/runs/{run['id']}/artifacts", 'artifacts')
    sources = {}
    for artifact in artifacts:
        match = re.fullmatch(r'gate-source-([1-9][0-9]*)', artifact.get('name', ''))
        if not match or artifact.get('expired'):
            continue
        source_attempt = int(match[1])
        require(source_attempt <= attempt, 'source artifact from a future attempt')
        require(source_attempt not in sources, 'ambiguous attempt source artifacts')
        sources[source_attempt] = provenance(api.source(artifact), run, number, head, source_attempt)
    require(sources, 'missing source artifact')
    source = sources[max(sources)]
    require(all(tuple(record[key] for key in ('base', 'head', 'merge')) ==
                tuple(source[key] for key in ('base', 'head', 'merge')) for record in sources.values()),
            'inconsistent same-run source triples')
    if main == source['base']:
        reason = None
    else:
        comparison = api.get(f"compare/{source['base']}...{main}?per_page=1")
        require(comparison.get('status') == 'ahead', 'unknown or rewritten main history')
        require(comparison.get('merge_base_commit', {}).get('sha') == source['base'], 'recorded base not main ancestor')
        require(type(comparison.get('total_commits')) is int and comparison['total_commits'] > 0, 'missing history')
        main_files = paths(comparison.get('files'))
        # Compare only returns files on page one, capped at 300 even when commits are paginated.
        require(0 < len(comparison['files']) < 300 and not comparison.get('truncated'),
                'missing or possibly truncated compare files')
        rows = api.pages(f'pulls/{number}/files', None)
        require(len(rows) == pr.get('changed_files') and len(rows) < 3000, 'incomplete PR file evidence')
        reason = conflict(main_files, paths(rows))
    # Re-read mutable pointers; advisory still cannot eliminate a race after this read.
    require(api.get(f'pulls/{number}')['head']['sha'] == head, 'PR head moved during check')
    require(api.get('git/ref/heads/main')['object']['sha'] == main, 'main moved during check')
    return reason


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('pr', type=int)
    parser.add_argument('--repo', default='arismoko/horizon')
    args = parser.parse_args()
    try:
        require(args.pr > 0, 'invalid PR number')
        reason = check(GitHub(args.repo), args.pr)
    except (KeyError, IndexError, AttributeError, TypeError, ValueError, OSError,
            subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        print(f'STALE: evidence unavailable: {error}')
        return 2
    print('STALE: ' + reason if reason else 'FRESH: current-head gate passes; no intervening conflict')
    return 1 if reason else 0


if __name__ == '__main__':
    raise SystemExit(main())
