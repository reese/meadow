//! Handler coverage for DSL-style class construction and load recording.

use meadow_handlers::build_registry;
use meadow_model::{KindData, ObjectKind, Ref};
use meadow_parser::parse_source;

#[test]
fn handles_struct_class_new_define_method_aliases_and_requires() {
    let source = r##"
class Base; end
class Child < Base; end
Point = Struct.new(:x, :y)
Empty = Struct.new
class StructChild < Struct.new(:a, :b); end
Derived = Class.new(Base)
AliasConst = Base

class Attrs
  attr :single
end

class Defined
  define_method(:dynamic) { |a, b| a }
  define_method("#{poison}_name") { }
  require_relative "other"
  require "json"
  autoload :Lazy, "lazy"
end
"##;
    let facts = parse_source("extensions.rb", source);
    let registry = build_registry(&[facts]);

    let point = registry.find("Point").expect("Point struct class");
    assert_eq!(point.kind, ObjectKind::Class);
    assert!(registry.find("Point#x").is_some());
    assert!(registry.find("Point#x=").is_some());
    assert!(registry.find("Point#y").is_some());
    let KindData::Namespace { superclass, .. } = &point.extra else {
        panic!("Point namespace");
    };
    assert!(matches!(superclass, Some(Ref::Proxy { name, .. }) if name == "Struct"));

    let empty = registry.find("Empty").expect("Empty struct class");
    assert_eq!(empty.kind, ObjectKind::Class);
    assert!(registry.find("Empty#x").is_none());

    let derived = registry.find("Derived").expect("Derived class");
    assert_eq!(derived.kind, ObjectKind::Class);
    let KindData::Namespace { superclass, .. } = &derived.extra else {
        panic!("Derived namespace");
    };
    assert!(matches!(superclass, Some(Ref::Resolved(_))));

    let dynamic = registry.find("Defined#dynamic").expect("dynamic method");
    assert!(dynamic.dynamic);
    let KindData::Method { params, .. } = &dynamic.extra else {
        panic!("dynamic method data");
    };
    assert_eq!(params.len(), 2);

    let alias = registry.find("AliasConst").expect("constant alias");
    let KindData::Constant { alias_of, .. } = &alias.extra else {
        panic!("constant data");
    };
    assert!(matches!(alias_of, Some(Ref::Resolved(_))));

    assert!(registry.find("StructChild#a").is_some());
    assert!(registry.find("StructChild#a=").is_some());
    assert!(registry.find("Attrs#single").is_some());
    assert!(registry.find("Attrs#single=").is_none());
    assert!(
        registry
            .objects()
            .iter()
            .all(|object| !object.path.to_string_lossy().contains("poison"))
    );

    assert_eq!(registry.requires().len(), 3);
    assert!(
        registry
            .requires()
            .iter()
            .any(|record| record.arguments[0].as_bytes() == b"\"json\"")
    );
}
