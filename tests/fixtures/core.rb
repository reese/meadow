# A top-level module.
module Greeter
  # Says hello to +name+.
  def hello(name)
    "Hello, #{name}"
  end
end

# A class with a superclass.
class Person < Object
  include Greeter

  # The person's name.
  attr_reader :name

  # Builds a person.
  def initialize(name, age = 0, *rest, active: true, **options, &block)
    @name = name
  end

  # Creates a person without calling +new+ directly.
  def self.create(name)
    new(name)
  end

  # A nickname.
  alias nickname name

  VERSION = "1.0"
  @@count = 0
end

module Outer
  module Inner
  end

  class Nested::Thing
  end
end
