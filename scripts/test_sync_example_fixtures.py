#!/usr/bin/env python3
"""Contract tests for the repository-only Golden AI Stream scenario."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = Path(__file__).with_name("sync-example-fixtures.py")
SPEC = importlib.util.spec_from_file_location("sync_example_fixtures", MODULE_PATH)
assert SPEC is not None
sync_example_fixtures = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = sync_example_fixtures
SPEC.loader.exec_module(sync_example_fixtures)


class GoldenScenarioContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scenario = json.loads(
            (ROOT / "examples/fixtures/golden-ai-stream.json").read_text(
                encoding="utf-8"
            )
        )

    def test_checked_in_scenario_is_valid(self) -> None:
        sync_example_fixtures.validate_scenario(self.scenario)

    def test_action_and_checkpoint_shape_errors_are_deterministic(self) -> None:
        cases = {
            "missing kind": lambda value: value["episodes"]["mainline"]["actions"][
                0
            ].pop("kind"),
            "unknown kind": lambda value: value["episodes"]["mainline"][
                "actions"
            ][0].__setitem__("kind", "paint"),
            "duplicate checkpoint": duplicate_checkpoint,
            "reordered finish": reorder_finish,
            "wrong source cursor": lambda value: value["episodes"]["mainline"][
                "actions"
            ][1].__setitem__("source_cursor", 1),
        }
        for message, mutate in cases.items():
            with self.subTest(case=message):
                value = json.loads(json.dumps(self.scenario))
                mutate(value)
                with self.assertRaisesRegex(
                    sync_example_fixtures.ScenarioError,
                    message,
                ):
                    sync_example_fixtures.validate_scenario(value)

    def test_unknown_fields_and_invalid_recovery_references_are_rejected(self) -> None:
        unknown = json.loads(json.dumps(self.scenario))
        unknown["episodes"]["mainline"]["actions"][0]["color"] = "teal"
        with self.assertRaisesRegex(
            sync_example_fixtures.ScenarioError,
            "unknown field.*color",
        ):
            sync_example_fixtures.validate_scenario(unknown)

        missing_snapshot = json.loads(json.dumps(self.scenario))
        recovery = missing_snapshot["episodes"]["recovery"]["actions"]
        next(
            action for action in recovery if action["kind"] == "recover_snapshot"
        )["snapshot"] = "missing-checkpoint"
        with self.assertRaisesRegex(
            sync_example_fixtures.ScenarioError,
            "unknown snapshot.*missing-checkpoint",
        ):
            sync_example_fixtures.validate_scenario(missing_snapshot)

    def test_check_detects_drift_without_rewriting(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            authority = root / "authority.json"
            destination = root / "copy.json"
            authority.write_bytes(b'{"authority":true}\n')
            destination.write_bytes(b'{"authority":false}\n')
            before = destination.read_bytes()

            with self.assertRaisesRegex(
                sync_example_fixtures.SyncError,
                "copy.json differs from authority.json",
            ):
                sync_example_fixtures.sync_fixture_copies(
                    authority,
                    (destination,),
                    check=True,
                )

            self.assertEqual(destination.read_bytes(), before)

    def test_write_mode_creates_byte_identical_copies(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            authority = root / "authority.json"
            destinations = (root / "nested" / "one.json", root / "two.json")
            authority.write_bytes(b'{"authority":true}\n')

            sync_example_fixtures.sync_fixture_copies(
                authority,
                destinations,
                check=False,
            )

            expected = authority.read_bytes()
            self.assertTrue(all(path.read_bytes() == expected for path in destinations))

    def test_repository_check_entrypoint_passes(self) -> None:
        result = subprocess.run(
            [sys.executable, str(MODULE_PATH), "--check"],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Golden AI Stream fixtures are synchronized", result.stdout)


def duplicate_checkpoint(value: dict[str, object]) -> None:
    actions = value["episodes"]["mainline"]["actions"]
    checkpoints = [action for action in actions if action["kind"] == "checkpoint"]
    checkpoints[1]["id"] = checkpoints[0]["id"]


def reorder_finish(value: dict[str, object]) -> None:
    actions = value["episodes"]["mainline"]["actions"]
    finish = next(action for action in actions if action["kind"] == "finish")
    actions.remove(finish)
    actions.insert(0, finish)


if __name__ == "__main__":
    unittest.main()
