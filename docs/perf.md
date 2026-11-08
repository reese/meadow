# Performance and robustness baselines

Recorded on macOS (Apple silicon) with a release build. Times are wall-clock
for the compiled `meadow` binary and exclude `cargo run` startup. These are
smoke-test baselines, not strict budgets.

## Real-gem and application corpus

Command:

```sh
cargo build --release -p meadow-cli
python3 scripts/corpus.py --repeat 5
```

Largest entries from `corpus/baseline.json` (median of 5 runs):

| Corpus entry | Kind         | Files | Methods | Wall time |
| ------------ | ------------ | ----: | ------: | --------: |
| rails        | gem monorepo | 1,470 |  15,563 |     100ms |
| discourse    | app          | 2,406 |  17,401 |     116ms |
| chatwoot     | app          | 1,028 |   7,358 |      61ms |
| mastodon     | app          | 1,371 |   7,479 |      60ms |
| rubocop      | gem          |   957 |   8,667 |      58ms |
| rack         | gem          |    50 |     607 |       9ms |

All 50 pinned entries complete without panics and total roughly 0.8s of
combined wall time. `corpus/baseline.json` stores milliseconds (median of
five runs for recorded baselines), since most entries are in the tens or low
hundreds of milliseconds. Peak RSS is about 43 MiB for Rails and 44 MiB for
Discourse on the cold `stats` path (`--no-save --no-cache`).

Cold builds stream parsed facts to the cache in batches instead of retaining
the whole event stream, and warm `stats` loads only a small diagnostics file
rather than the parse-facts cache.

Representative larger runs:

| Project   | Files | Classes | Methods |  Wall time | Max RSS |
| --------- | ----: | ------: | ------: | ---------: | ------: |
| Rails     | 1,470 |   1,642 |  15,563 |      0.10s |  43 MiB |
| Discourse | 2,406 |   2,506 |  17,401 |      0.12s |  44 MiB |
| Chatwoot  | 1,028 |     914 |   7,358 |      0.06s |  22 MiB |

## Warm-cache runs

With the registry cache populated, `stats` only compares file metadata and
loads the cached registry plus the small diagnostics side file; the
parse-facts cache is not read.

| Project   | Warm-cache `stats` | Peak RSS |
| --------- | -----------------: | -------: |
| Rails     |           ~30–40ms |   28 MiB |
| Discourse |           ~30–40ms |   29 MiB |

Changed files fall back to content hashing; unchanged files are not re-read.

## Remaining opportunities

- Skip resolution-warning string construction when `--no-warn-unresolved` is set.
- Per-class dependency tracking to avoid rebuilding page contexts for unchanged
  classes, not just skipping writes.
- Optional allocator swap (mimalloc/jemalloc) for allocation-heavy parse/handle
  runs.
- Further reduce `SourceText`/path allocations in hot handler/resolution loops.

## HTML generation

Discourse (2,406 files, ~54 MiB of HTML): ~0.4s to re-render with a warm
registry cache, and ~0.25s when every page is unchanged. Cold runs take ~0.6s
and are dominated by filesystem writes. Page contexts are built and written in
bounded batches, keeping peak RSS around 156 MiB while re-rendering, 107 MiB
when pages are unchanged, and 163 MiB on a cold run.

## Synthetic corpus

`crates/meadow-handlers/tests/scale.rs` generates 200 files with 600 classes
and 12,000 documented methods, then builds a registry in memory. The test
asserts the object count and a generous 30-second ceiling; the current run is
far below that.
