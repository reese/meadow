#!/usr/bin/env python3
"""Run meadow over the pinned real-world corpus and compare object counts."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "corpus" / "manifest.toml"
BASELINE = ROOT / "corpus" / "baseline.json"
COUNT_RE = {
    "files": re.compile(r"^Files:\s+(\d+)", re.MULTILINE),
    "modules": re.compile(r"^Modules:\s+(\d+)", re.MULTILINE),
    "classes": re.compile(r"^Classes:\s+(\d+)", re.MULTILINE),
    "constants": re.compile(r"^Constants:\s+(\d+)", re.MULTILINE),
    "class_variables": re.compile(r"^Class variables:\s*(\d+)", re.MULTILINE),
    "methods": re.compile(r"^Methods:\s+(\d+)", re.MULTILINE),
    "attributes": re.compile(r"^Attributes:\s+(\d+)", re.MULTILINE),
}


def load_manifest() -> list[dict]:
    with MANIFEST.open("rb") as handle:
        return tomllib.load(handle)["gem"]


def fetch(gem: dict, workdir: Path) -> Path | None:
    destination = workdir / "src" / gem["name"]
    if destination.exists():
        current = subprocess.run(
            ["git", "-C", str(destination), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
        )
        if current.returncode == 0 and current.stdout.strip() == gem["sha"]:
            return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        subprocess.run(["git", "init", "-q", str(destination)], check=True)
        subprocess.run(
            ["git", "-C", str(destination), "remote", "add", "origin", gem["repo"]],
            check=True,
        )
    for command in (
        ["git", "-C", str(destination), "fetch", "--depth", "1", "origin", gem["sha"]],
        ["git", "-C", str(destination), "checkout", "-q", "FETCH_HEAD"],
    ):
        result = subprocess.run(command, capture_output=True, text=True)
        if result.returncode != 0:
            print(f"  fetch failed: {result.stderr.strip()}", file=sys.stderr)
            return None
    return destination


def meadow_binary() -> list[str]:
    for candidate in (ROOT / "target" / "release" / "meadow", ROOT / "target" / "debug" / "meadow"):
        if candidate.is_file():
            return [str(candidate)]
    return ["cargo", "run", "-q", "-p", "meadow-cli", "--"]


def run_stats(gem: dict, source: Path, repeat: int = 1) -> dict | None:
    patterns = [str(source / pattern) for pattern in gem.get("paths", ["lib/**/*.rb"])]
    command = [
        *meadow_binary(),
        "stats",
        "--no-save",
        "--no-cache",
        "--no-warn-unresolved",
        "--db",
        str(ROOT / ".corpus" / "db"),
        *patterns,
    ]
    samples = []
    output = None
    for _ in range(max(1, repeat)):
        started = time.perf_counter()
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
        samples.append(int(round((time.perf_counter() - started) * 1000)))
        if result.returncode != 0:
            print(f"  meadow failed ({result.returncode}): {result.stderr.strip()}", file=sys.stderr)
            return None
        output = result.stdout
    samples.sort()
    elapsed_ms = samples[len(samples) // 2]
    counts = {}
    for key, pattern in COUNT_RE.items():
        match = pattern.search(output)
        if not match:
            print(f"  missing stat {key}: {output}", file=sys.stderr)
            return None
        counts[key] = int(match.group(1))
    counts["milliseconds"] = elapsed_ms
    counts["sha"] = gem["sha"]
    return counts


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workdir", default=str(ROOT / ".corpus"))
    parser.add_argument("--limit", type=int)
    parser.add_argument("--only", action="append")
    parser.add_argument("--update-baseline", action="store_true")
    parser.add_argument("--repeat", type=int, default=1)
    args = parser.parse_args()

    workdir = Path(args.workdir)
    workdir.mkdir(parents=True, exist_ok=True)
    gems = load_manifest()
    if args.only:
        wanted = set(args.only)
        gems = [gem for gem in gems if gem["name"] in wanted]
    if args.limit:
        gems = gems[: args.limit]

    baseline = {}
    if BASELINE.exists():
        baseline = json.loads(BASELINE.read_text())

    results = {}
    failures = []
    for gem in gems:
        print(f"== {gem['name']} {gem['sha'][:12]}")
        source = fetch(gem, workdir)
        if source is None:
            failures.append(gem["name"])
            continue
        counts = run_stats(gem, source, args.repeat)
        if counts is None:
            failures.append(gem["name"])
            continue
        results[gem["name"]] = counts
        if args.update_baseline:
            baseline[gem["name"]] = counts
            BASELINE.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n")
        print(
            f"  files={counts['files']} classes={counts['classes']} methods={counts['methods']} "
            f"time={counts['milliseconds']}ms"
        )

        previous = baseline.get(gem["name"])
        if previous and not args.update_baseline:
            for key in ("files", "modules", "classes", "constants", "class_variables", "methods", "attributes"):
                if previous.get(key) != counts[key]:
                    print(f"  REGRESSION {key}: {previous.get(key)} -> {counts[key]}")
                    failures.append(gem["name"])
            baseline_ms = previous.get(
                "milliseconds",
                int(round(previous.get("seconds", 0) * 1000)),
            )
            if counts["milliseconds"] > baseline_ms * 3 + 2000:
                print(
                    f"  RUNTIME {counts['milliseconds']}ms exceeds baseline budget "
                    f"({baseline_ms}ms * 3 + 2000ms)"
                )

    if args.update_baseline:
        baseline.update(results)
        BASELINE.write_text(json.dumps(baseline, indent=2, sort_keys=True) + "\n")
        print(f"updated {BASELINE}")
    else:
        missing = [name for name in results if name not in baseline]
        if missing:
            print(f"new corpus entries without baselines: {', '.join(missing)}")

    if failures:
        print(f"FAILED: {', '.join(sorted(set(failures)))}", file=sys.stderr)
        return 1
    print(f"corpus ok: {len(results)} gems")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
