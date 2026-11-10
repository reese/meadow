# typed: false
class RbiExample
  # Original source docs.
  sig { params(name: Symbol).returns(Symbol) }
  def greet(name)
    name
  end

  def undocumented(value)
    value
  end
end

# A source class that an RBI augments.
class RbiOnly
end
