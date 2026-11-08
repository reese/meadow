# A person.
class Person
  # The person's name.
  attr_reader :name

  # Create a person.
  # @param [String] name the name
  def initialize(name, age = 0)
  end

  # Greet someone.
  # @return [String] existing return description
  def greet(name)
  end

  private

  def secret; end
end

# A top-level module.
module Greeter
end
