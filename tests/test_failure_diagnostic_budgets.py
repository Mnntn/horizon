"""Synthetic failure payloads only; no game or prepared-data execution."""
import contextlib
import io
import unittest
from unittest.mock import patch

from tools.ci import ignored_tests as ci


class Budgets(unittest.TestCase):
    def render(self, output):
        with contextlib.redirect_stdout(io.StringIO()) as log:
            ci.failure_diagnostics(output)
        return log.getvalue()

    def test_120_failures_all_have_identity_and_first_safe_context(self):
        output = ''.join(f"test suite::case_{i} ... FAILED\n"
                         f"thread 'suite::case_{i}' panicked at .local/private.rs:1:1:\n"
                         "assertion `left == right` failed: POISON\n left: 96\n right: 93\n"
                         for i in range(120))
        log = self.render(output)
        for i in range(120):
            self.assertIn(f'failed test: suite::case_{i}\n', log)
        self.assertEqual(log.count('assertion equality failed'), 120)
        self.assertEqual(log.count('assertion left: 96'), 120)
        self.assertIn('120 shown; 0 failing tests not shown', log)
        self.assertNotIn('POISON', log)
        self.assertNotIn('.local', log)

    def test_total_budget_counts_occurrences_including_duplicate_names(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 200):
            log = self.render('test repeated ... FAILED\n' * 120)
        payload, metadata = log.split('failure diagnostics: ')
        shown = payload.count('failed test:')
        self.assertLessEqual(len(payload.encode()), 200)
        self.assertIn(f'{shown} shown; {120 - shown} failing tests not shown', metadata)
        self.assertIn('total byte budget 200', metadata)

    def test_multibyte_identity_and_per_test_budget(self):
        log = self.render('test ' + '界' * 400 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertIn('shortened: per-test byte budget', payload)

    def test_missing_and_poisoned_context_are_explicit(self):
        log = self.render("test absent ... FAILED\ntest missing ... FAILED\n"
                          "thread 'missing' panicked at .local/private.rs:1:1:\n"
                          "test poisoned ... FAILED\n"
                          "thread 'poisoned' panicked at .local/private.rs:1:1:\n"
                          "secret token POISON\n")
        self.assertIn('panic context missing', log)
        self.assertIn('panic reason missing', log)
        self.assertIn('custom panic; payload withheld', log)
        self.assertNotIn('POISON', log)

    def test_forged_and_mixed_identity_lines_do_not_publish_tokens(self):
        log = self.render('test safe::case ... FAILED\n'
                          'test ghp_PRIVATE ... FAILED\n'
                          'test safe::ghp_PRIVATE ... FAILED\n'
                          'test safe::case/PRIVATE ... FAILED\n')
        self.assertIn('failed test: safe::case\n', log)
        self.assertEqual(log.count('failed test: identity withheld'), 3)
        self.assertNotIn('PRIVATE', log)
        self.assertIn('4 shown; 0 failing tests not shown', log)

    def test_true_utf8_boundary(self):
        # 15 prefix bytes + 600 UTF-8 identity bytes; clipping crosses a codepoint.
        log = self.render('test ' + '界' * 200 + ' ... FAILED\n')
        payload = log.split('failure diagnostics: ')[0]
        self.assertIn('界', payload)
        self.assertNotIn('\\u', payload)
        self.assertNotIn('\ufffd', payload)
        self.assertLessEqual(len(payload.encode()), 500)
        self.assertGreater(len(payload.encode()), 497)

    def test_duplicate_names_use_first_context_for_each_occurrence(self):
        log = self.render("test same ... FAILED\nthread 'same' panicked at private.rs:1:1:\n"
                          "attempt to divide by zero\n"
                          "test same ... FAILED\nthread 'same' panicked at private.rs:2:1:\n"
                          "attempt to add with overflow\n")
        self.assertEqual(log.count('failed test: same'), 2)
        self.assertEqual(log.count('attempt to divide by zero'), 2)
        self.assertNotIn('attempt to add with overflow', log)

    def test_exact_total_edge_uses_independent_record_size(self):
        record = '  failed test: edge\n  panic context missing from captured output\n'
        for budget, expected in ((len(record.encode()) * 2, 2),
                                 (len(record.encode()) * 2 - 1, 1)):
            with patch.object(ci, 'FAILURE_TOTAL_BYTES', budget):
                log = self.render('test edge ... FAILED\n' * 3)
            self.assertIn(f'{expected} shown; {3 - expected} failing tests not shown', log)

    def test_flood_bounds_and_tracked_source_lookup_is_once(self):
        output = ''.join(f"test c{i} ... FAILED\nthread 'c{i}' panicked at crates/x.rs:1:1:\n"
                         "attempt to divide by zero\n" for i in range(20))
        ci.tracked_rust_paths.cache_clear()
        import subprocess
        with patch.object(ci, 'DIAGNOSTIC_LINES', 30), patch.object(ci, 'DIAGNOSTIC_CONTEXTS', 2), \
                patch.object(ci.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '')) as git:
            log = self.render(output)
        self.assertEqual(git.call_count, 1)
        self.assertIn('30 lines skipped', log)
        self.assertIn('8 contexts skipped', log)
        ci.tracked_rust_paths.cache_clear()
        with patch.object(ci, 'DIAGNOSTIC_LINE_CHARS', 50):
            log = self.render('x' * 100 + '\ntest ok ... FAILED\n')
        self.assertIn('1 lines skipped', log)

    def test_policy_annotations_share_one_budget_across_runs(self):
        with patch.object(ci, 'FAILURE_TOTAL_BYTES', 100), \
                patch.object(ci, 'known_failures', return_value={}), \
                patch.object(ci, 'run', return_value={'ghp_PRIVATE': False, 'safe': False}), \
                contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(ci.main(['one', 'two']), 1)
        self.assertNotIn('PRIVATE', log.getvalue())
        annotations = ''.join(line + '\n' for line in log.getvalue().splitlines() if line.startswith('::error::'))
        self.assertLessEqual(len(annotations.encode()), 100)
        self.assertIn('problems not shown (shared total byte budget)', log.getvalue())
