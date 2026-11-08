#!/usr/bin/env ruby
# frozen_string_literal: true

# A free-standing directive block.
#
# @!macro [new] my_macro

# A documented class.
# @note Attached comments keep their second line.
class Documented
=begin
A method documented with an embdoc block.
=end
  def from_embdoc; end

  #   indented code example
  #     puts "hi"
  def indented
  end

  attr_accessor :value # Trailing comment for an attribute.
end
