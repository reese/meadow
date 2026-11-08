# A module with mixins, constants, and module functions.
module Utilities
  # Included helper.
  module Helpers
  end

  # Extended helper.
  module ClassHelpers
  end

  # Prepended helper.
  module Before
  end

  include Helpers
  extend ClassHelpers
  prepend Before

  # A module constant.
  LIMIT = 10

  # A module variable.
  @@registry = {}

  # A module function.
  def util(x)
    x
  end

  module_function :util
end

# A reopened class.
class Widget
  # The widget name.
  attr_accessor :name

  # Read-only id.
  attr_reader :id

  # Write-only secret.
  attr_writer :secret

  # A public method.
  def public_method; end

  protected

  def protected_method; end

  private

  def private_method; end

  public

  private :public_method
  protected :protected_method

  private def inline_private(x); end
  protected def inline_protected; end

  alias alias_method public_method
  alias_method :alias_by_call, :public_method

  COLOR = "red"
  @@count = 0
end

# Reopen Widget.
class Widget
  def extra; end
end

# Compact namespace.
class Widget::Part
  def initialize; end
end

# A class with class methods and a singleton body.
class Factory
  def self.build; end

  class << self
    def configure; end
  end
end

# Module function no-arg form.
module MathHelpers
  module_function

  def double(x)
    x * 2
  end
end
