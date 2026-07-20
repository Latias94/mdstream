#!/usr/bin/env python3
"""Validate and synchronize the repository-only Golden AI Stream scenario."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Iterable, Mapping, Sequence


ROOT = Path(__file__).resolve().parents[1]
AUTHORITY = ROOT / "examples" / "fixtures" / "golden-ai-stream.json"
SCHEMA = ROOT / "examples" / "fixtures" / "golden-ai-stream.schema.json"
DESTINATIONS = (
    ROOT / "mdstream" / "examples" / "fixtures" / "golden-ai-stream.json",
    ROOT / "mdstream-merman" / "examples" / "fixtures" / "golden-ai-stream.json",
    ROOT / "bindings" / "dart" / "example" / "fixtures" / "golden_ai_stream.json",
    ROOT / "bindings" / "flutter" / "example" / "assets" / "golden_ai_stream.json",
)
SCENARIO_SCHEMA = "mdstream.example-scenario/1"
OBSERVATIONS = frozenset(
    (
        "pending_source",
        "provisional_inline",
        "provisional_code_fence",
        "provisional_code_block",
        "provisional_mermaid_fence",
        "provisional_mermaid_block",
        "provisional_citation_definition",
        "stable_code_block",
        "stable_mermaid_block",
        "unresolved_citation",
        "resolved_citation",
        "semantic_correction",
        "finalized",
    )
)


class ScenarioError(RuntimeError):
    """The hand-authored example scenario violates its repository contract."""


class SyncError(RuntimeError):
    """A generated scenario copy is missing or differs from its authority."""


def validate_scenario(value: object) -> None:
    scenario = _object(value, "scenario")
    _fields(
        scenario,
        "scenario",
        ("$schema", "schema", "id", "description", "episodes", "expected"),
    )
    _equal(scenario["$schema"], "./golden-ai-stream.schema.json", "$schema")
    _equal(scenario["schema"], SCENARIO_SCHEMA, "schema")
    _equal(scenario["id"], "golden-ai-stream", "id")
    _string(scenario["description"], "description")

    episodes = _object(scenario["episodes"], "episodes")
    _fields(episodes, "episodes", ("mainline", "recovery"))
    mainline = _object(episodes["mainline"], "episodes.mainline")
    _fields(mainline, "episodes.mainline", ("schedules", "actions"))

    actions = _list(mainline["actions"], "episodes.mainline.actions")
    source, checkpoints, stage_boundaries = _validate_mainline(actions)
    _validate_schedules(mainline["schedules"], source, stage_boundaries)
    _validate_expected(scenario["expected"], source)
    _validate_recovery(episodes["recovery"], checkpoints)


def sync_fixture_copies(
    authority: Path,
    destinations: Iterable[Path],
    *,
    check: bool,
) -> None:
    expected = authority.read_bytes()
    errors: list[str] = []
    for destination in destinations:
        if check:
            if not destination.is_file():
                errors.append(
                    f"{_relative(destination)} is missing; expected a copy of "
                    f"{_relative(authority)}"
                )
            elif destination.read_bytes() != expected:
                errors.append(
                    f"{_relative(destination)} differs from {_relative(authority)}"
                )
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(expected)
    if errors:
        raise SyncError("\n".join(errors))


def _validate_mainline(
    actions: Sequence[object],
) -> tuple[str, frozenset[str], tuple[int, ...]]:
    if len(actions) < 3:
        raise ScenarioError("mainline must contain append/checkpoint pairs and finish")
    source_parts: list[str] = []
    checkpoint_ids: set[str] = set()
    append_ids: set[str] = set()
    boundaries: list[int] = []
    source_cursor = 0
    expected_kind = "append"

    for index, raw_action in enumerate(actions):
        path = f"episodes.mainline.actions[{index}]"
        action = _object(raw_action, path)
        if "kind" not in action:
            raise ScenarioError(f"missing kind at {path}")
        kind = action["kind"]
        if kind not in ("append", "checkpoint", "finish"):
            raise ScenarioError(f"unknown kind `{kind}` at {path}")
        if kind != expected_kind:
            if kind == "finish" and expected_kind == "append" and index == len(actions) - 1:
                pass
            elif kind == "finish":
                raise ScenarioError(
                    f"reordered finish at {path}; finish must follow the final checkpoint"
                )
            else:
                raise ScenarioError(
                f"reordered {kind} at {path}; expected {expected_kind}"
                )

        if kind == "append":
            _fields(action, path, ("kind", "id", "chunk"))
            action_id = _string(action["id"], f"{path}.id")
            if action_id in append_ids:
                raise ScenarioError(f"duplicate append id `{action_id}` at {path}")
            append_ids.add(action_id)
            chunk = _string(action["chunk"], f"{path}.chunk")
            source_parts.append(chunk)
            source_cursor += len(chunk.encode("utf-8"))
            boundaries.append(source_cursor)
            expected_kind = "checkpoint"
            continue

        if kind == "checkpoint":
            _fields(
                action,
                path,
                ("kind", "id", "scope", "source_cursor", "observations"),
            )
            checkpoint_id = _string(action["id"], f"{path}.id")
            if checkpoint_id in checkpoint_ids:
                raise ScenarioError(f"duplicate checkpoint `{checkpoint_id}` at {path}")
            checkpoint_ids.add(checkpoint_id)
            if action["scope"] not in ("schedule_local", "boundary_invariant"):
                raise ScenarioError(f"unknown checkpoint scope at {path}")
            cursor = _integer(action["source_cursor"], f"{path}.source_cursor")
            if cursor != source_cursor:
                raise ScenarioError(
                    f"wrong source cursor at {path}: expected {source_cursor}, got {cursor}"
                )
            observations = _list(action["observations"], f"{path}.observations")
            if not observations:
                raise ScenarioError(f"{path}.observations must not be empty")
            if len(set(observations)) != len(observations):
                raise ScenarioError(f"duplicate observation at {path}")
            unknown = sorted(set(observations) - OBSERVATIONS)
            if unknown:
                raise ScenarioError(f"unknown observation `{unknown[0]}` at {path}")
            expected_kind = "append"
            continue

        _fields(action, path, ("kind", "id", "observations"))
        if index != len(actions) - 1:
            raise ScenarioError(
                f"reordered finish at {path}; finish must be the final action"
            )
        _equal(action["id"], "finalized", f"{path}.id")
        final_observations = set(_list(action["observations"], f"{path}.observations"))
        expected_final_observations = {
            "finalized",
            "resolved_citation",
            "semantic_correction",
            "stable_mermaid_block",
        }
        if final_observations != expected_final_observations:
            raise ScenarioError(
                "finish observations must declare finalized citation correction and stable Mermaid"
            )
        expected_kind = "done"

    if expected_kind != "done":
        raise ScenarioError("mainline must end with finish after a checkpoint")
    return "".join(source_parts), frozenset(checkpoint_ids), tuple(boundaries)


def _validate_schedules(
    raw_schedules: object,
    source: str,
    stage_boundaries: Sequence[int],
) -> None:
    schedules = _list(raw_schedules, "episodes.mainline.schedules")
    expected = (
        ("whole", "whole", False),
        ("stage-aligned", "stage_aligned", True),
        ("adversarial", "byte_cuts", True),
    )
    if len(schedules) != len(expected):
        raise ScenarioError("mainline must declare whole, stage-aligned, and adversarial schedules")
    for index, (raw_schedule, contract) in enumerate(zip(schedules, expected)):
        path = f"episodes.mainline.schedules[{index}]"
        schedule = _object(raw_schedule, path)
        schedule_id, kind, compatible = contract
        required = ("id", "kind", "checkpoint_compatible")
        if kind == "byte_cuts":
            required += ("cuts",)
        _fields(schedule, path, required)
        _equal(schedule["id"], schedule_id, f"{path}.id")
        _equal(schedule["kind"], kind, f"{path}.kind")
        _equal(
            schedule["checkpoint_compatible"],
            compatible,
            f"{path}.checkpoint_compatible",
        )

    cuts = _list(schedules[2]["cuts"], "episodes.mainline.schedules[2].cuts")
    if any(type(cut) is not int for cut in cuts):
        raise ScenarioError("adversarial cuts must be integers")
    if cuts != sorted(set(cuts)):
        raise ScenarioError("adversarial cuts must be strictly increasing and unique")
    source_bytes = len(source.encode("utf-8"))
    if not cuts or cuts[0] <= 0 or cuts[-1] >= source_bytes:
        raise ScenarioError("adversarial cuts must lie inside the UTF-8 source")
    missing = sorted(set(stage_boundaries[:-1]) - set(cuts))
    if missing:
        raise ScenarioError(
            "checkpoint-compatible adversarial schedule is missing stage boundaries: "
            + ", ".join(str(value) for value in missing)
        )
    encoded = source.encode("utf-8")
    for cut in cuts:
        try:
            encoded[:cut].decode("utf-8")
        except UnicodeDecodeError as error:
            raise ScenarioError(f"adversarial cut {cut} splits a UTF-8 scalar") from error


def _validate_expected(raw_expected: object, source: str) -> None:
    path = "expected"
    expected = _object(raw_expected, path)
    _fields(
        expected,
        path,
        (
            "final_source",
            "lifecycle",
            "all_nodes_stable",
            "node_kinds",
            "resource_kinds",
            "code_languages",
        ),
    )
    if expected["final_source"] != source:
        raise ScenarioError("expected.final_source does not match concatenated append chunks")
    _equal(expected["lifecycle"], "finalized", "expected.lifecycle")
    _equal(expected["all_nodes_stable"], True, "expected.all_nodes_stable")
    for field in ("node_kinds", "resource_kinds", "code_languages"):
        values = _list(expected[field], f"expected.{field}")
        if not values or any(not isinstance(value, str) or not value for value in values):
            raise ScenarioError(f"expected.{field} must contain non-empty strings")
        if values != sorted(set(values)):
            raise ScenarioError(f"expected.{field} must be sorted and unique")


def _validate_recovery(raw_recovery: object, checkpoints: frozenset[str]) -> None:
    path = "episodes.recovery"
    recovery = _object(raw_recovery, path)
    _fields(recovery, path, ("trace", "actions"))
    _equal(recovery["trace"], "stage-aligned", f"{path}.trace")
    actions = _list(recovery["actions"], f"{path}.actions")
    expected_kinds = (
        "apply_change",
        "apply_change",
        "recover_snapshot",
        "apply_change",
        "apply_change",
        "recover_snapshot",
        "reset",
    )
    if len(actions) != len(expected_kinds):
        raise ScenarioError("recovery must cover two gap paths and one reset")
    for index, (raw_action, expected_kind) in enumerate(zip(actions, expected_kinds)):
        action_path = f"{path}.actions[{index}]"
        action = _object(raw_action, action_path)
        if "kind" not in action:
            raise ScenarioError(f"missing kind at {action_path}")
        kind = action["kind"]
        if kind not in ("apply_change", "recover_snapshot", "reset"):
            raise ScenarioError(f"unknown kind `{kind}` at {action_path}")
        if kind != expected_kind:
            raise ScenarioError(
                f"reordered recovery action at {action_path}: expected {expected_kind}"
            )
        if kind == "apply_change":
            _fields(
                action,
                action_path,
                ("kind", "target", "change_ordinal", "expect", "continuity"),
            )
            _integer(action["change_ordinal"], f"{action_path}.change_ordinal")
            continue
        if kind == "recover_snapshot":
            _fields(
                action,
                action_path,
                ("kind", "target", "snapshot", "expect", "continuity"),
            )
            snapshot = _string(action["snapshot"], f"{action_path}.snapshot")
            if snapshot not in checkpoints:
                raise ScenarioError(f"unknown snapshot `{snapshot}` at {action_path}")
            continue
        _fields(
            action,
            action_path,
            ("kind", "target", "snapshot", "expect_epoch", "continuity"),
        )

    contracts = (
        ("same-floor-replica", 0, "applied", "retained"),
        ("same-floor-replica", 2, "gap", "awaiting_snapshot"),
        ("same-floor-replica", "inline-provisional", "recovered", "retained_same_floor"),
        ("advanced-replica", 0, "applied", "retained"),
        ("advanced-replica", 2, "gap", "awaiting_snapshot"),
        (
            "advanced-replica",
            "mermaid-body-provisional",
            "recovered",
            "replaced_advanced",
        ),
        ("producer", "finalized", 2, "new_epoch"),
    )
    for index, (action, contract) in enumerate(zip(actions, contracts)):
        reference = action.get("change_ordinal", action.get("snapshot"))
        actual = (action.get("target"), reference, action.get("expect", action.get("expect_epoch")), action.get("continuity"))
        if actual != contract:
            raise ScenarioError(
                f"recovery contract mismatch at {path}.actions[{index}]: "
                f"expected {contract}, got {actual}"
            )


def _object(value: object, path: str) -> Mapping[str, object]:
    if not isinstance(value, dict):
        raise ScenarioError(f"{path} must be an object")
    return value


def _list(value: object, path: str) -> list[object]:
    if not isinstance(value, list):
        raise ScenarioError(f"{path} must be an array")
    return value


def _string(value: object, path: str) -> str:
    if not isinstance(value, str) or not value:
        raise ScenarioError(f"{path} must be a non-empty string")
    return value


def _integer(value: object, path: str) -> int:
    if type(value) is not int or value < 0:
        raise ScenarioError(f"{path} must be a non-negative integer")
    return value


def _equal(value: object, expected: object, path: str) -> None:
    if value != expected:
        raise ScenarioError(f"{path} must equal {expected!r}, got {value!r}")


def _fields(
    value: Mapping[str, object],
    path: str,
    required: Sequence[str],
) -> None:
    required_set = set(required)
    missing = sorted(required_set - value.keys())
    if missing:
        raise ScenarioError(f"missing field `{missing[0]}` at {path}")
    unknown = sorted(value.keys() - required_set)
    if unknown:
        raise ScenarioError(f"unknown field `{unknown[0]}` at {path}")


def _relative(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.name


def load_and_validate(path: Path) -> object:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ScenarioError(f"cannot read {_relative(path)}: {error}") from error
    validate_scenario(value)
    return value


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="fail on missing or drifted generated copies without writing files",
    )
    args = parser.parse_args(argv)
    try:
        load_and_validate(AUTHORITY)
        json.loads(SCHEMA.read_text(encoding="utf-8"))
        sync_fixture_copies(AUTHORITY, DESTINATIONS, check=args.check)
    except (OSError, UnicodeError, json.JSONDecodeError, ScenarioError, SyncError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    action = "synchronized" if args.check else "updated"
    print(f"Golden AI Stream fixtures are {action}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
