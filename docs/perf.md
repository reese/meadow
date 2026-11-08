# Performance and robustness baselines

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

Cold-cache runs:

| Project   | Files | Classes | Methods | Wall time | Max RSS |
| --------- | ----: | ------: | ------: | --------: | ------: |
| Rails     | 1,470 |   1,642 |  15,563 |     0.10s |  43 MiB |
| Discourse | 2,406 |   2,506 |  17,401 |     0.12s |  44 MiB |
| Chatwoot  | 1,028 |     914 |   7,358 |     0.06s |  22 MiB |

## Warm-cache runs

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
