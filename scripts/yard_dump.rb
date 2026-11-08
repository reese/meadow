require 'yard'
require 'json'

YARD::Registry.clear
Dir.glob(File.join(ARGV[0], '**', '*.rb')).sort.each do |file|
  begin
    YARD::Parser::SourceParser.parse(file)
  rescue StandardError
  end
end

objects = YARD::Registry.all.map do |object|
  { 'path' => object.path, 'type' => object.type.to_s, 'tags' => object.tags.map(&:tag_name) }
end
puts JSON.generate(objects)
