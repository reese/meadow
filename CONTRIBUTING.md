# Contributing

## Development

The project is a Cargo workspace:

| Crate              | Responsibility                                         |
| ------------------ | ------------------------------------------------------ |
| `meadow-model`     | Code object model, registry, serialization             |
| `meadow-parser`    | Ruby and RBS parsing to a shared event stream          |
| `meadow-handlers`  | Raw events to code objects                             |
| `meadow-docstring` | Docstrings, tags, type expressions, directives, macros |
| `meadow-resolve`   | Namespace, inheritance, and link resolution            |
| `meadow-markup`    | Markdown/RDoc rendering                                |
| `meadow-html`      | Templates, search index, site writer                   |
| `meadow-plugins`   | Plugin trait, Sorbet, and ActiveRecord plugins         |
| `meadow-cli`       | The `meadow` binary                                    |

Run the test suite:

```sh
cargo test --workspace
```

Expected parser and registry outputs live under `tests/expected/` and are compared
byte for byte. Regenerate them after intentional pipeline changes:

```sh
UPDATE_EXPECTED=1 cargo test -p meadow-parser expected_event_streams
UPDATE_EXPECTED=1 cargo test -p meadow-handlers expected_registry_output
UPDATE_EXPECTED=1 cargo test -p meadow-handlers expected_plugin_registries
UPDATE_EXPECTED=1 cargo test -p meadow-html matches_the_expected_person_page
```

Useful development tools:

```sh
# Inspect a file's parse event stream
cargo run -p meadow-parser --example dump_events -- tests/fixtures/core.rb

# Inspect the RBS syntax tree for a signature file
cargo run -p meadow-parser --example dump_rbs -- tests/fixtures/rbs/core.rbs

# Render a site from the test fixtures
cargo test -p meadow-html renders_a_complete_site

# Run the pinned corpus and compare with real YARD
python3 scripts/corpus.py --limit 10
python3 scripts/compare_yard.py path/to/gem/lib

# Preview docs for a pinned corpus gem
python3 scripts/preview_corpus.py rack
```
