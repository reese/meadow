# Parses tagged input.
class Tagged
  # Parses things.
  #
  # Longer description.
  #
  # @param [String, nil] name the name
  # @param count [Integer] the count
  # @option options [String] :format ("json") output format
  # @return [Array<String>] the results
  # @raise [ArgumentError] when invalid
  # @yield [item] each item
  # @yieldparam item [String] item name
  # @yieldreturn [void] nothing
  # @example Basic usage
  #   Tagged.new.parse("x")
  # @see Other
  # @note A note
  # @api private
  # @deprecated Use something else
  # @since 1.0
  # @todo Later
  # @author Someone
  # @version 2.0
  # @abstract Override me
  # @private
  # @unknown something
  # @overload parse(input)
  #   @param [String] input the input
  #   @return [Array<String>] parsed
  def parse(name, count = 1, options = {})
  end

  # @param [Nope] missing not a parameter
  # @return [Array<] broken type
  def warnings
  end
end

# Inherits method docs.
class Child < Tagged
  def parse(name, count = 1, options = {})
  end
end
