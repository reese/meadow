#!/usr/bin/env python3
"""Non-blocking comparison of YARD and meadow object paths/tag names."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("path", help="Ruby source directory")
    parser.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()

    yard = subprocess.run(
        ["ruby", str(ROOT / "scripts" / "yard_dump.rb"), args.path],
        capture_output=True,
        text=True,
    )
    if yard.returncode != 0:
        print(f"YARD failed: {yard.stderr.strip()}", file=sys.stderr)
        return 0
    yard_objects = {item["path"]: item for item in json.loads(yard.stdout)}

    binary = ROOT / "target" / "release" / "meadow"
    meadow_command = (
        [str(binary)]
        if binary.is_file()
        else ["cargo", "run", "-q", "-p", "meadow-cli", "--"]
    )
    meadow = subprocess.run(
        [
            *meadow_command,
            "list",
            "--format",
            "json",
            "--no-cache",
            "--no-save",
            str(Path(args.path) / "**" / "*.rb"),
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if meadow.returncode != 0:
        print(f"meadow failed: {meadow.stderr.strip()}", file=sys.stderr)
        return 0
    meadow_objects = {item["path"]: item for item in json.loads(meadow.stdout)}

    only_yard = sorted(set(yard_objects) - set(meadow_objects))
    only_meadow = sorted(set(meadow_objects) - set(yard_objects))
    common = sorted(set(yard_objects) & set(meadow_objects))

    print(f"YARD: {len(yard_objects)} objects, meadow: {len(meadow_objects)} objects")
    print(f"common: {len(common)}, only YARD: {len(only_yard)}, only meadow: {len(only_meadow)}")
    for path in only_yard[: args.limit]:
        print(f"  only YARD: {path}")
    for path in only_meadow[: args.limit]:
        print(f"  only meadow: {path}")

    tag_differences = 0
    for path in common:
        yard_tags = set(yard_objects[path].get("tags", []))
        meadow_tags = set(meadow_objects[path].get("tags", []))
        if yard_tags != meadow_tags:
            tag_differences += 1
            if tag_differences <= args.limit:
                print(f"  tags differ for {path}: YARD={sorted(yard_tags)} meadow={sorted(meadow_tags)}")
    print(f"objects with tag differences: {tag_differences}")
    print("Comparison is non-blocking; triage differences into docs/deviations.md.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
