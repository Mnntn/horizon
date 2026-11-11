"""The sample gate uses the same immutable revision on PR and main events."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.ci import check_machine_sample as gate


class MachineSampleGateTests(unittest.TestCase):
    def test_paired_and_main_events_build_the_same_pin_and_cleanup(self):
        for event in ({}, {"pull_request": {"body": "Sample commit: " + gate.PIN}}):
            with self.subTest(event=event), tempfile.TemporaryDirectory() as directory:
                event_file = Path(directory) / "event.json"
                event_file.write_text(json.dumps(event))
                visited = []

                def check(destination, revision):
                    self.assertTrue(destination.is_dir())
                    self.assertEqual(revision, gate.PIN)
                    visited.append(destination)

                with patch.dict(os.environ, {"GITHUB_EVENT_PATH": str(event_file), "GITHUB_STEP_SUMMARY": ""}), patch.object(gate, "check", check):
                    gate.main()
                self.assertFalse(visited[0].exists())

    def test_body_cannot_override_the_committed_pin(self):
        with tempfile.TemporaryDirectory() as directory:
            event_file = Path(directory) / "event.json"
            event_file.write_text(json.dumps({"pull_request": {"body": "Sample commit: " + "0" * 40}}))
            with patch.dict(os.environ, {"GITHUB_EVENT_PATH": str(event_file)}), patch.object(gate, "check") as check:
                with self.assertRaises(SystemExit):
                    gate.main()
                check.assert_not_called()
