# `meadow`

`meadow` is a delightfully fast, [YARD](https://yardoc.org/)-compatible documentation generator for Ruby.

## Installation

`meadow` is still in early development, so building from source is the only supported installation method.

```sh
git clone https://github.com/reese/meadow
cd meadow
cargo install --path crates/meadow-cli
```

This installs the `meadow` binary. You can also build a release binary without
installing it:

```sh
cargo build --release -p meadow-cli
# binary at target/release/meadow
```

## Quick start

From the root of a Ruby project:

```sh
meadow doc
```

By default this reads Ruby/RBS files from `lib/`, `app/`, and `ext/`, reads RBS
signatures from `lib/`, `app/`, and `sig/`, writes the site to `doc/`, and uses
`README.md` as the landing page when present. Open `doc/index.html` to view the
result.

Pass files, directories, or globs explicitly if your sources live elsewhere:

```sh
meadow doc app/models lib/**/*.rb
```

## Commands

| Command  |                                                         |
| -------- | ------------------------------------------------------- |
| `doc`    | Build the HTML documentation site.                      |
| `stats`  | Print object counts and documentation coverage.         |
| `list`   | List objects, optionally filtered with `--query`.       |
| `server` | Serve the site locally and rebuild when sources change. |

Examples:

```sh
# Documentation coverage, including undocumented object paths
meadow stats
meadow stats --list-undoc

# Find public methods with API tags
meadow list --query "object.type == :method && object.visibility == :public"
meadow list --format json --query "@api.text == 'public'"

# Preview on http://127.0.0.1:8808 with live reload
meadow server
meadow server --port 9000
```

## Configuration

Options can be passed on the command line, stored in `.meadow.toml`, or stored in
`.yardopts` (YARD syntax). `.yardopts` is applied after `.meadow.toml`, and
command-line flags override both.

```toml
# .meadow.toml
title = "My Project"
output-dir = "doc"
markup = "markdown"          # or "rdoc"
exclude = ["vendor/**", "spec/**"]
plugins = ["sorbet", "activerecord"]
readme = "README.md"
private = false
protected = true
```

For a full list of flags, see `meadow help doc`.

The registry, parse cache, and manifest live under `.meadow/`; add that directory
to your `.gitignore`.

## Plugins

Two plugins are built in:

- `sorbet` — reads Sorbet `sig` blocks, `T::Struct`/`T::Props` definitions, and
  `T.let` annotations.
- `activerecord` — infers model columns, associations, scopes, and enums from
  `db/schema.rb` and ActiveRecord superclasses.

```sh
meadow doc --plugin sorbet --plugin activerecord
```

ActiveRecord settings can be tuned in `.meadow.toml`:

```toml
plugins = ["activerecord"]
activerecord-schema = "db/schema.rb"
activerecord-models = ["ApplicationRecord"]
```

## YARD compatibility

`meadow` is designed as a drop-in replacement for common YARD workflows. It
reads `.yardopts`, supports YARD tags, directives, and macros, and renders a
similar class/method/file structure. See
[docs/deviations.md](docs/deviations.md) for known differences.

## Performance

`meadow` parses large codebases in parallel and caches unchanged files between
runs. Projects with thousands of files and tens of thousands of methods complete
in well under a second, often under 100ms. See [docs/perf.md](docs/perf.md) for measured baselines.
