import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / 'tools/refactors/split_mod_vocabulary.py'


def load():
    spec = importlib.util.spec_from_file_location('split_mod_vocabulary', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AuditTests(unittest.TestCase):
    def run_audit(self, facade_files):
        module = load()
        with tempfile.TemporaryDirectory() as root:
            root = pathlib.Path(root)
            for name in facade_files:
                path = root / module.OLD / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('')
            refuse = mock.Mock(side_effect=AssertionError('the audit must not run git'))
            with mock.patch.object(module, 'ROOT', root), \
                    mock.patch.object(subprocess, 'run', refuse), \
                    mock.patch.object(subprocess, 'check_output', refuse), \
                    mock.patch.object(sys, 'argv', ['split_mod_vocabulary.py', '--audit']):
                module.main()

    def test_audit_of_a_moved_tree_passes_and_moves_nothing(self):
        self.run_audit(['lib.rs', 'wire/wire_guest.rs'])

    def test_audit_rejects_vocabulary_left_in_the_facade(self):
        with self.assertRaises(SystemExit) as raised:
            self.run_audit(['lib.rs', 'wire/wire_guest.rs', 'hooks/model_layers.rs'])
        self.assertIn('hooks/model_layers.rs', str(raised.exception))


if __name__ == '__main__':
    unittest.main()
