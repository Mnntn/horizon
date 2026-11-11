"""Local git histories supply independent commit/file evidence; no network or Rust."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('merge_fresh', Path(__file__).resolve().parents[1] / 'tools/ci/merge_fresh.py')
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)


class Evidence:
    def __init__(self, base, head, main, files, pr_files):
        self.pr = {'state': 'open', 'base': {'ref': 'main'}, 'head': {'sha': head}, 'changed_files': len(pr_files)}
        self.main = main
        self.run = {'id': 12, 'run_attempt': 2, 'head_sha': head, 'event': 'pull_request', 'pull_requests': [{'number': 7}]}
        self.record = dict(schema_version=1, pr=7, run_id=12, attempt=2, base=base, head=head, merge='f' * 40)
        self.jobs = [{'name': name, 'status': 'completed', 'conclusion': 'success'}
                     for name in ('lint', 'verify', 'launcher', 'pool-watch')]
        self.artifacts = [{'id': 33, 'name': 'gate-source-2', 'expired': False}]
        self.comparison = {'status': 'ahead', 'merge_base_commit': {'sha': base}, 'total_commits': 1, 'files': files}
        self.pr_files = pr_files

    def get(self, endpoint):
        if endpoint == 'pulls/7':
            return copy.deepcopy(self.pr)
        if endpoint == 'git/ref/heads/main':
            return {'object': {'sha': self.main}}
        if endpoint.startswith('compare/'):
            return self.comparison
        raise AssertionError(endpoint)

    def pages(self, endpoint, key):
        if endpoint.startswith('actions/workflows/'):
            return [self.run]
        if endpoint.endswith('/jobs'):
            assert '/attempts/2/' in endpoint
            return self.jobs
        if endpoint.endswith('/artifacts'):
            return self.artifacts
        if endpoint == 'pulls/7/files':
            return self.pr_files
        raise AssertionError(endpoint)

    def source(self, artifact):
        return self.record


class Freshness(unittest.TestCase):
    def history(self, main_path, pr_path, main_content='main change', pr_content='PR change'):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            def git(*args):
                return subprocess.check_output(['git', '-C', temp, *args], stderr=subprocess.DEVNULL, text=True).strip()
            git('init', '-q')
            git('config', 'user.name', 'Fixture')
            git('config', 'user.email', 'fixture@example.invalid')
            def commit(path, content):
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content)
                git('add', '--', path)
                git('commit', '-qm', content)
                return git('rev-parse', 'HEAD')
            base = commit('README.md', 'base')
            head = commit(pr_path, pr_content)
            git('checkout', '-q', '--detach', base)
            main = commit(main_path, main_content)
            def rows(tip):
                return [{'filename': name, 'status': 'modified'} for name in git('diff', '--name-only', base, tip).splitlines()]
            return Evidence(base, head, main, rows(main), rows(head))

    def test_slot_row_2989_vs_2972(self):
        # #2989 added owned/borrowed record slot rows while #2972 added a registration row.
        api = self.history('crates/horizon-mod-api-gen/slots/portizon.txt',
                           'crates/horizon-client/slots/villager.txt',
                           'listed call owned and borrowed records', 'register villager profession')
        self.assertIn('shared contract', m.check(api, 7))

    def test_wire_pins_3013_vs_2983(self):
        # #3013 froze fieldless enum identities while #2983 retired Wire derives.
        api = self.history('tools/fixtures/wire_enum_layouts.json',
                           'crates/horizon-mod-vocab/src/hooks/entity.rs',
                           '{"baseline": "fieldless enum hash"}', 'retire non-Wire enum')
        self.assertIn('shared contract', m.check(api, 7))

    def test_disjoint(self):
        self.assertIsNone(m.check(self.history('crates/a/src/a.rs', 'crates/b/src/b.rs'), 7))

    def test_same_crate(self):
        self.assertIn('same crate', m.check(self.history('crates/a/src/a.rs', 'crates/a/src/b.rs'), 7))

    def test_current_base_contract_is_fresh_but_advanced_main_is_not(self):
        api = self.history('crates/a/src/a.rs', 'tools/fixtures/wire_enum_layouts.json')
        self.assertIn('shared contract', m.check(api, 7))
        api.main = api.record['base']
        self.assertIsNone(m.check(api, 7))

    def test_tooling_and_workflows_are_file_scoped(self):
        self.assertIsNone(m.check(self.history('tools/ci/ignored_tests.py', 'tools/ci/nightly.py'), 7))
        self.assertIsNone(m.check(self.history('crates/a/src/a.rs', '.github/workflows/check.yml'), 7))
        self.assertIn('same file', m.check(self.history('tools/ci/nightly.py', 'tools/ci/nightly.py'), 7))
        self.assertIn('same file', m.check(self.history('.github/workflows/check.yml', '.github/workflows/check.yml'), 7))

    def test_bad_evidence(self):
        good = self.history('crates/a/src/a.rs', 'crates/b/src/b.rs')
        mutations = [lambda a: a.record.update(head='0'*40), lambda a: a.record.update(attempt=1),
                     lambda a: a.record.update(pr=8), lambda a: a.record.update(base='not sha'),
                     lambda a: a.run.update(head_sha='0'*40), lambda a: a.run.update(event='workflow_dispatch'),
                     lambda a: a.run.update(pull_requests=[]), lambda a: a.jobs.clear(),
                     lambda a: a.jobs[0].update(conclusion='failure'), lambda a: a.jobs[0].update(status='queued'),
                     lambda a: a.artifacts.clear(), lambda a: a.artifacts.append(dict(a.artifacts[0])),
                     lambda a: a.artifacts[0].update(name='gate-source-1'),
                     lambda a: a.comparison.update(files=[{'filename': str(i)} for i in range(300)]),
                     lambda a: a.comparison.update(files=None), lambda a: a.comparison.update(status='diverged'),
                     lambda a: a.pr.update(changed_files=3)]
        for mutate in mutations:
            api = copy.deepcopy(good)
            mutate(api)
            with self.subTest(mutate=mutate), self.assertRaises(m.Unavailable):
                m.check(api, 7)

    def test_optional_skips_and_required_skips(self):
        api = self.history('crates/a/src/a.rs', 'crates/b/src/b.rs')
        api.jobs[2]['conclusion'] = api.jobs[3]['conclusion'] = 'skipped'
        self.assertIsNone(m.check(api, 7))
        api.jobs[0]['conclusion'] = 'skipped'
        with self.assertRaises(m.Unavailable):
            m.check(api, 7)

    def test_deleted_and_renamed_contract(self):
        self.assertIn('tools/fixtures/wire_enum_layouts.json', m.paths([{'filename': 'tools/fixtures/wire_enum_layouts.json', 'status': 'removed'}]))
        names = m.paths([{'filename': 'elsewhere/new.rs', 'status': 'renamed', 'previous_filename': 'crates/a/slots/old.rs'}])
        self.assertIn('shared contract', m.conflict(names, {'crates/b/src/b.rs'}))
        self.assertIn('shared contract', m.conflict(m.paths([{'filename': 'crates/a/slots/new.rs', 'status': 'renamed', 'previous_filename': 'elsewhere/old.rs'}]), {'x'}))

    def test_gh_read_only_and_pagination(self):
        with patch.object(m.subprocess, 'run') as run:
            run.return_value.stdout = json.dumps({'items': [] , 'total_count': 0}).encode()
            self.assertEqual(m.GitHub('arismoko/horizon').pages('endpoint', 'items'), [])
            self.assertEqual(run.call_args.args[0], ['gh', 'api', 'repos/arismoko/horizon/endpoint?per_page=100&page=1'])
        api = m.GitHub('arismoko/horizon')
        with patch.object(api, 'get', return_value={'items': [], 'total_count': 1}), self.assertRaises(m.Unavailable):
            api.pages('endpoint', 'items')


if __name__ == '__main__':
    unittest.main()
