#!/usr/bin/env python3
"""Generate and open documentation for one pinned corpus entry.

Examples:
    python3 scripts/preview_corpus.py rack
    python3 scripts/preview_corpus.py discourse --plugin activerecord
    python3 scripts/preview_corpus.py rails --no-open
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from corpus import ROOT, fetch, load_manifest  # noqa: E402

WORKDIR = ROOT / ".corpus"
DOCS = ROOT / "target" / "corpus-docs"


def ensure_binary() -> Path:
    binary = ROOT / "target" / "release" / "meadow"
    if not binary.is_file():
        print("building release binary...", file=sys.stderr)
        subprocess.run(
            ["cargo", "build", "--release", "-p", "meadow-cli"],
            cwd=ROOT,
            check=True,
        )
    return binary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name", help="corpus entry name from corpus/manifest.toml")
    parser.add_argument(
        "--plugin",
        action="append",
        default=[],
        help="built-in plugin to enable (repeatable: sorbet, activerecord)",
    )
    parser.add_argument("--no-open", action="store_true", help="generate but do not open a browser")
    parser.add_argument("--server", action="store_true", help="run the live-reload server instead of opening")
    parser.add_argument("--port", type=int, default=8808, help="server port (default: 8808)")
    args = parser.parse_args()

    gems = {gem["name"]: gem for gem in load_manifest()}
    gem = gems.get(args.name)
    if gem is None:
        print(f"unknown corpus entry '{args.name}'", file=sys.stderr)
        print("available: " + ", ".join(sorted(gems)), file=sys.stderr)
        return 2

    source = fetch(gem, WORKDIR)
    if source is None:
        print(f"failed to fetch {args.name}", file=sys.stderr)
        return 1

    binary = ensure_binary()
    output_dir = DOCS / args.name
    cache_dir = (WORKDIR / "cache" / args.name).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    output_dir = output_dir.resolve()
    patterns = [str(source / pattern) for pattern in gem.get("paths", ["lib/**/*.rb"])]

    command = [
        str(binary),
        "doc",
        *patterns,
        "--output-dir",
        str(output_dir),
        "--db",
        str(cache_dir),
        "--title",
        args.name,
    ]
    for plugin in args.plugin:
        command.extend(["--plugin", plugin])

    if args.server:
        command = [
            *binary_args_for_server(binary, patterns, output_dir, cache_dir, args.name, args.plugin),
            "--port",
            str(args.port),
        ]
        print("serving at http://127.0.0.1:%d" % args.port)
        try:
            subprocess.run(command, cwd=source, check=True)
        except KeyboardInterrupt:
            return 130
        except subprocess.CalledProcessError as error:
            return 130 if error.returncode < 0 else error.returncode
        return 0

    result = subprocess.run(command, cwd=source)
    if result.returncode != 0:
        return result.returncode

    index = output_dir / "index.html"
    print(f"documentation: {index}")
    if not args.no_open:
        opener = "open" if sys.platform == "darwin" else "xdg-open"
        try:
            subprocess.run([opener, str(index)], check=False)
        except FileNotFoundError:
            print(f"could not find '{opener}'; open {index} manually", file=sys.stderr)
    return 0


def binary_args_for_server(
    binary: Path,
    patterns: list[str],
    output_dir: Path,
    cache_dir: Path,
    title: str,
    plugins: list[str],
) -> list[str]:
    command = [
        str(binary),
        "server",
        *patterns,
        "--output-dir",
        str(output_dir),
        "--db",
        str(cache_dir),
        "--title",
        title,
    ]
    for plugin in plugins:
        command.extend(["--plugin", plugin])
    return command


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\nstopped", file=sys.stderr)
        raise SystemExit(130) from None
