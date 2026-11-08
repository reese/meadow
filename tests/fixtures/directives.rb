# @!macro [new] standalone
#   @return [String] standalone value

# Directives demonstration.
class Directives
  # @!method generated(a, b = 1)
  #   @param [Integer] a first
  #   @return [Integer] result
  #
  # @!attribute [rw] exposed
  #   The exposed attribute.
  #
  # @!scope class
  # @!method self.class_generated(x)
  #   @param [String] x input
  # @!scope instance
  #
  # @!visibility private
  # @!method hidden_generated
  # @!visibility public
  #
  # @!group Utilities
  # @!method grouped
  # @!endgroup
  #
  def documented_method(a); end

  # @!parse
  #   def from_parse; end

  # @!macro [attach] property
  #   @!method $0
  #   @return [$1] the $1 value
  property :thing

  property :widget, :extra
end
