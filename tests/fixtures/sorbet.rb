# typed: strict
class SorbetExample
  extend T::Sig

  # Has a signature.
  sig { params(name: String, age: T.nilable(Integer)).returns(T::Array[String]) }
  def tagged(name, age = nil)
  end

  sig { void }
  def no_return; end

  sig { abstract }
  def abstract_method; end

  sig(:final) { override.returns(Integer) }
  def final_method; end

  sig { returns(T.proc.params(item: String).returns(Integer)) }
  def with_yield; end

  sig { params(flag: T::Boolean).returns(T.any(String, Symbol)) }
  def booleans(flag); end

  # Explicit docs win over signatures.
  # @param [Symbol] name explicit type
  # @return [String] explicit result
  sig { params(name: Integer).returns(Integer) }
  def explicit(name)
  end

  sig { void }
  def initialize
    @value = T.let(1, Integer)
  end

  CONST = T.let("value", String)
end

class Point < T::Struct
  const :x, Integer
  prop :label, T.nilable(String)
end

class Color < T::Enum
  enums do
    Red = new
    Blue = new
  end
end
