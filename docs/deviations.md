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
- Unresolved constants and class names remain proxies. They render without links
  unless an external proxy URL is available. Exact proxy entries win; otherwise
  constants are linked through their parent namespace (`File::SEPARATOR` uses
  the `File` URL plus a `#SEPARATOR` anchor, and top-level constants use an
  `Object` URL).
- Sorbet: `.rbi` files are supported as type-signature overlays. Declarations
  merge into matching source objects, so RBI `sig` blocks supply parameter and
  return types without replacing source documentation. RBI-only declarations
  are also included. By default, generated third-party RBIs under
  `sorbet/rbi/gems/` are not discovered; pass them explicitly if you want to
  document them too. `T::Props` beyond `prop`/`const` basics remain out of
  scope.
- RBS support is declaration-level: classes, modules, methods, attributes,
  mixins, aliases, constants, instance variables, and class variables are
  merged into the registry. Interfaces, type aliases, class/module aliases, and
  complex type forms are not rendered as objects; unsupported type expressions
  are displayed as raw RBS text.
- Macro expansion is bounded and does not re-run arbitrary Ruby.
- The HTML output is similar, but it's not intended to be exactly equivalent to YARD
