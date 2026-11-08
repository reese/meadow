# A module for call events.
module Calls
  # Include another module.
  include Enumerable

  # Defines readers.
  attr_reader :first, :second

  # Defines a writer.
  attr_writer :third

  # Defines an accessor.
  attr_accessor :fourth

  private

  def hidden; end

  public

  # A macro-style DSL call.
  my_dsl :thing, option: true do |value|
    helper(value)
  end

  define_method(:dynamic) do |x|
    x
  end

  if condition
    def conditional_method; end
  end
end
