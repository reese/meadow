//! Parser coverage for common Ruby declaration and expression forms.

use meadow_parser::{
    AliasEvent, CallEvent, ClassOpenEvent, Event, FileFacts, ModuleOpenEvent,
    SingletonClassOpenEvent, parse_source,
};

fn parse(source: &str) -> FileFacts {
    parse_source("coverage.rb", source)
}

fn method_names(facts: &FileFacts) -> Vec<String> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::MethodDef(method) => Some(method.name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn call_names(facts: &FileFacts) -> Vec<String> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Call(call) => Some(call.name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn calls<'a>(facts: &'a FileFacts, name: &str) -> Vec<&'a CallEvent> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Call(call) if call.name == name => Some(call),
            _ => None,
        })
        .collect()
}

fn aliases(facts: &FileFacts) -> Vec<&AliasEvent> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Alias(alias) => Some(alias),
            _ => None,
        })
        .collect()
}

fn classes(facts: &FileFacts) -> Vec<&ClassOpenEvent> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::ClassOpen(class) => Some(class),
            _ => None,
        })
        .collect()
}

fn modules(facts: &FileFacts) -> Vec<&ModuleOpenEvent> {
    facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::ModuleOpen(module) => Some(module),
            _ => None,
        })
        .collect()
}

#[test]
fn parses_alias_forms() {
    let facts = parse(
        r#"
module Wrapper
  def original; end

  alias copy original
  alias :quoted :original
  alias :'spaced name' :original
  alias :question? :original
  alias :[] :original
  alias :[]= :original
  alias :-@ :original
  alias :<< :original

  alias_method :call_copy, :original
  alias_method :"call_spaced", :"original"
end
"#,
    );

    let aliases = aliases(&facts);
    let names: Vec<_> = aliases
        .iter()
        .map(|alias| alias.new_name.to_string_lossy().into_owned())
        .collect();
    for expected in [
        "copy",
        "quoted",
        "spaced name",
        "question?",
        "[]",
        "[]=",
        "-@",
        "<<",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "missing alias {expected}: {names:?}"
        );
    }

    let alias_method_calls = calls(&facts, "alias_method");
    assert_eq!(alias_method_calls.len(), 2);
    assert_eq!(
        alias_method_calls[0].arguments[0].literal.as_ref().unwrap(),
        "call_copy"
    );
}

#[test]
fn parses_attribute_forms() {
    let facts = parse(
        r#"
module Writable
  attr_writer :only_write
end

class Item
  class << self
    attr_accessor :count
  end

  attr :legacy, true
  attr_reader :first, :second
  attr_writer :secret
  attr_accessor "label"
end
"#,
    );

    assert_eq!(calls(&facts, "attr_writer").len(), 2);
    assert_eq!(calls(&facts, "attr_reader").len(), 1);
    assert_eq!(calls(&facts, "attr_accessor").len(), 2);
    assert_eq!(calls(&facts, "attr").len(), 1);

    let readers = calls(&facts, "attr_reader");
    let literals: Vec<_> = readers[0]
        .arguments
        .iter()
        .filter_map(|argument| argument.literal.as_ref())
        .map(|literal| literal.to_string_lossy().into_owned())
        .collect();
    assert_eq!(literals, ["first", "second"]);

    assert!(
        facts
            .events
            .iter()
            .any(|event| matches!(event, Event::SingletonClassOpen(SingletonClassOpenEvent { receiver, .. }) if receiver == "self"))
    );
}

#[test]
fn parses_conditional_method_definitions() {
    let facts = parse(
        r"
class Conditional
  if enabled?
    def available; end
  else
    def unavailable; end
  end

  def trailing; end if enabled?
  def trailing_unless; end unless enabled?

  unless enabled?
    def disabled_branch; end
  end

  def outer
    def nested; end
  end
end
",
    );

    let names = method_names(&facts);
    for expected in [
        "available",
        "unavailable",
        "trailing",
        "trailing_unless",
        "disabled_branch",
        "outer",
        "nested",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing method {expected}: {names:?}"
        );
    }
}

#[test]
fn parses_nested_and_reopened_namespaces() {
    let facts = parse(
        r"
module Outer
  module Inner; end

  class Compact::Thing < Base; end

  class << self
    def build; end
  end
end

module ::Rooted
end

class Reopened; end
class Reopened; end
",
    );

    let module_names: Vec<_> = modules(&facts)
        .iter()
        .map(|module| module.name.to_string_lossy().into_owned())
        .collect();
    assert!(module_names.iter().any(|name| name == "Outer"));
    assert!(module_names.iter().any(|name| name == "Inner"));
    assert!(module_names.iter().any(|name| name == "::Rooted"));

    let class_names: Vec<_> = classes(&facts)
        .iter()
        .map(|class| class.name.to_string_lossy().into_owned())
        .collect();
    assert!(class_names.iter().any(|name| name == "Compact::Thing"));
    assert_eq!(
        classes(&facts)
            .iter()
            .filter(|class| class.name == "Reopened")
            .count(),
        2
    );

    let compact = classes(&facts)
        .into_iter()
        .find(|class| class.name == "Compact::Thing")
        .expect("compact class");
    assert_eq!(compact.superclass.as_ref().unwrap(), "Base");
}

#[test]
fn parses_constants_and_variables() {
    let facts = parse(
        r#"
module Configuration
  VERSION = "1.0"
  MULTILINE = FIRST +
    SECOND
  @@registry = {}
  @cache = nil
end

Point = Struct.new(:x, :y)
Record = Data.define(:name) do
  def label; end
end
"#,
    );

    let constants: Vec<_> = facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::ConstantWrite(constant) => Some(constant.path.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for expected in ["VERSION", "MULTILINE", "Point", "Record"] {
        assert!(
            constants.iter().any(|path| path == expected),
            "missing constant {expected}: {constants:?}"
        );
    }

    assert!(facts.events.iter().any(|event| matches!(
        event,
        Event::ClassVariableWrite(variable) if variable.name == "@@registry"
    )));
    assert!(facts.events.iter().any(|event| matches!(
        event,
        Event::InstanceVariableWrite(variable) if variable.name == "@cache"
    )));
}

#[test]
fn parses_mixins_and_visibility_changes() {
    let facts = parse(
        r"
module Helpers; end

module Feature
  include Helpers
  extend Helpers
  prepend Helpers
end

class Visible
  def public_one; end

  private
  def private_one; end

  protected
  def protected_one; end

  public
  def public_two; end

  private def inline_private; end
end
",
    );

    for name in ["include", "extend", "prepend"] {
        let entries = calls(&facts, name);
        assert_eq!(entries.len(), 1, "expected one {name} call");
        assert_eq!(entries[0].arguments[0].source, "Helpers");
    }

    let visibility_calls = call_names(&facts);
    for name in ["private", "protected", "public"] {
        assert!(
            visibility_calls.iter().any(|call| call == name),
            "missing visibility call {name}: {visibility_calls:?}"
        );
    }

    let names = method_names(&facts);
    for expected in [
        "public_one",
        "private_one",
        "protected_one",
        "public_two",
        "inline_private",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing method {expected}"
        );
    }
}

#[test]
fn parses_unusual_method_names() {
    let facts = parse(
        r"
class Operators
  def ==(other); end
  def /(other); end
  def [](key); end
  def []=(key, value); end
  def |(other); end
  def =~(other); end
  def -@; end
  def +@; end
  def ~@; end
  def &(other); end
  def %(other); end
  def ^(other); end
  def *(other); end
  def <<(other); end
  def allowed?; end
  def valid!; end
end
",
    );

    let names = method_names(&facts);
    for expected in [
        "==", "/", "[]", "[]=", "|", "=~", "-@", "+@", "~", "&", "%", "^", "*", "<<", "allowed?",
        "valid!",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing method {expected}: {names:?}"
        );
    }
}

#[test]
fn parses_calls_blocks_and_error_handling() {
    let facts = parse(
        r#"
class Worker
  def perform(items)
    items.each { |item| process(item) }
    raise ArgumentError, "missing items" if items.empty?
  end

  def process(item)
    transform(item)
  end
end
"#,
    );

    let names = call_names(&facts);
    for expected in ["each", "process", "raise", "transform"] {
        assert!(
            names.iter().any(|name| name == expected),
            "missing call {expected}: {names:?}"
        );
    }

    let each = calls(&facts, "each")[0];
    assert!(each.block.is_some());
    assert!(method_names(&facts).iter().any(|name| name == "perform"));
}

#[test]
fn parses_directive_comments_and_private_constants() {
    let facts = parse(
        r"
module Endpoints
  Hidden = 1
  private_constant :Hidden

  # Declare a new endpoint.
  # @!method fetch(id)
  #   @param [Integer] id
  #   @return [String]
  endpoint :fetch
end
",
    );

    let endpoint = calls(&facts, "endpoint");
    assert_eq!(endpoint.len(), 1);
    let docstring = endpoint[0].docstring.as_ref().expect("directive docstring");
    assert!(docstring.text.contains("@!method fetch(id)"));

    let private_constant = calls(&facts, "private_constant");
    assert_eq!(private_constant.len(), 1);
    assert_eq!(
        private_constant[0].arguments[0].literal.as_ref().unwrap(),
        "Hidden"
    );
}
