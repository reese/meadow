# Known deviations from YARD

This list is non-exhaustive. The comparison harness (`scripts/compare_yard.py`)
reports differences for triage; they are not treated as failures.

- `meadow` does not yet support a plugin system, though it's on the roadmap!
- `meadow` defaults to Markdown instead of RDoc, and we only support a limited
  subset of RDoc syntax. Textile is not supported at all.
- YARD plugin/theme/ERB compatibility is out of scope.
- `meadow` does not evaluate Ruby, though it attempts to approximate some common Ruby constructs:
  - `define_method` and dynamically-built method names are only documented when
    statically visible in the AST.
  - `Struct.new`/`Class.new` constants are handled without executing Ruby.
  - ActiveRecord table/model inference uses names and `db/schema.rb`, not a
    live database or Rails initialization.
- Unresolved constants and class names remain proxies and render without links.
- Sorbet: RBI files and `T::Props` beyond `prop`/`const` basics are out of scope.
- RBS support is declaration-level: classes, modules, methods, attributes,
  mixins, aliases, constants, instance variables, and class variables are
  merged into the registry. Interfaces, type aliases, class/module aliases, and
  complex type forms are not rendered as objects; unsupported type expressions
  are displayed as raw RBS text.
- Macro expansion is bounded and does not re-run arbitrary Ruby.
- The HTML output is similar, but it's not intended to be exactly equivalent to YARD
