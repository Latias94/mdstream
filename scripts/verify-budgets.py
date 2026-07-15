#!/usr/bin/env python3
import argparse
import hashlib
import json
import subprocess
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
CONFORMANCE_TIMEOUT_SECONDS = 300
METADATA_TIMEOUT_SECONDS = 120


def load_json(relative_path: str) -> dict:
    path = REPOSITORY_ROOT / relative_path
    with path.open(encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise SystemExit(f"{relative_path} must contain a JSON object")
    return value


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def verify_fixture() -> None:
    streaming = load_json("conformance/budgets/streaming.json")
    fixture = streaming.get("provenance", {}).get("fixture")
    require(isinstance(fixture, dict), "calibration fixture provenance is required")
    relative_path = fixture.get("path")
    require(isinstance(relative_path, str) and relative_path, "calibration fixture path is required")
    fixture_path = (REPOSITORY_ROOT / relative_path).resolve()
    try:
        fixture_path.relative_to(REPOSITORY_ROOT.resolve())
    except ValueError:
        raise SystemExit("calibration fixture must remain inside the repository") from None
    require(fixture_path.is_file(), f"calibration fixture does not exist: {relative_path}")
    fixture_bytes = fixture_path.read_bytes()
    require(
        hashlib.sha256(fixture_bytes).hexdigest() == fixture.get("sha256"),
        "calibration fixture SHA-256 drifted",
    )
    require(len(fixture_bytes) == fixture.get("bytes"), "calibration fixture byte count drifted")
    try:
        fixture_text = fixture_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(f"calibration fixture is not UTF-8: {error}") from error
    require(len(fixture_text) == fixture.get("chunks"), "calibration fixture character count drifted")


def verify_contracts() -> None:
    verify_fixture()
    run_checked(
        [
            "cargo",
            "test",
            "-p",
            "mdstream-conformance",
            "--test",
            "budget_contract",
        ],
        "canonical budget conformance",
        CONFORMANCE_TIMEOUT_SECONDS,
    )
    print("Canonical budget contracts and calibration fixture are valid")


def verify_negative_merman() -> None:
    output = run_checked(
        ["cargo", "metadata", "--format-version", "1"],
        "Cargo metadata dependency resolution",
        METADATA_TIMEOUT_SECONDS,
        capture_output=True,
    )
    metadata = json.loads(output.stdout)
    packages = {package["id"]: package for package in metadata["packages"]}
    resolve = metadata.get("resolve")
    require(isinstance(resolve, dict), "cargo metadata dependency resolution is unavailable")
    nodes = {node["id"]: node for node in resolve["nodes"]}
    pending = list(metadata.get("workspace_default_members", []))
    require(bool(pending), "cargo metadata did not report default workspace members")
    reachable = set()
    while pending:
        package_id = pending.pop()
        if package_id in reachable:
            continue
        reachable.add(package_id)
        node = nodes.get(package_id)
        require(node is not None, f"cargo metadata omitted dependency node: {package_id}")
        for dependency in node.get("deps", []):
            kinds = dependency.get("dep_kinds", [])
            if not kinds or any(kind.get("kind") in (None, "normal", "build") for kind in kinds):
                pending.append(dependency["pkg"])
    matches = sorted(
        packages[package_id]["name"]
        for package_id in reachable
        if "merman" in packages[package_id]["name"].lower()
    )
    require(not matches, f"default workspace dependency graph contains Merman: {matches}")
    print("Default workspace dependency graph contains no Merman package")


def run_checked(
    command: list[str],
    label: str,
    timeout_seconds: int,
    *,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=capture_output,
            text=True,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired as error:
        raise SystemExit(f"{label} timed out after {timeout_seconds} seconds") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        suffix = f": {detail}" if detail else ""
        raise SystemExit(f"{label} failed with exit code {error.returncode}{suffix}") from error


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify mdstream budget contracts")
    parser.add_argument("--contracts", action="store_true")
    parser.add_argument("--negative-merman", action="store_true")
    args = parser.parse_args()
    run_all = not args.contracts and not args.negative_merman
    if run_all or args.contracts:
        verify_contracts()
    if run_all or args.negative_merman:
        verify_negative_merman()


if __name__ == "__main__":
    main()
