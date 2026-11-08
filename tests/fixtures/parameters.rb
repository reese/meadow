class Parameters
  def required(a, b); end

  def optional(a, b = 2); end

  def with_rest(a, *rest, z); end

  def with_keywords(a:, b: 2, **opts); end

  def with_block(&blk); end

  def forwarding(...); end

  def anonymous(*, **, &); end

  def no_keywords(**nil); end

  def endless(x) = x * 2

  def [](key); end

  def []=(key, value); end

  def self.build(name:); end

  def self.shout; end

  def object.method(value); end
end
