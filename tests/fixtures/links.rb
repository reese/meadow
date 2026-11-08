# Base docs.
class Base
  # Base method docs.
  # @return [String] result
  def base_method; end
end

# See style.
class Derived < Base
  # (see Base#base_method)
  def inherited_docs; end

  # Has {Base#base_method links} and {Base#base_method}.
  # Also {https://example.com Example} and {file:README.md#intro File}.
  # Inline `{Base#base_method}` stays.
  #
  # {include:Base#base_method}
  def linked; end
end

# Unresolved link.
class Missing
  # See {NoSuchThing#nope here}.
  def broken; end
end
