//! Prints a summary of the ruby-rbs AST for one signature file.

use std::fs;

use ruby_rbs::node::{
    AliasNode, ClassNode, CommentNode, ConstantNode, IncludeNode, MethodDefinitionKind,
    MethodDefinitionNode, ModuleNode, Node, SignatureNode, TypeNameNode,
};

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_rbs FILE");
    let source = fs::read_to_string(&path).expect("read file");
    let signature = ruby_rbs::node::parse(&source).expect("parse RBS");
    dump_signature(&signature, &source);
}

fn dump_signature(signature: &SignatureNode<'_>, source: &str) {
    for declaration in signature.declarations().iter() {
        dump_node(&declaration, source, 0);
    }
}

fn dump_node(node: &Node<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    match node {
        Node::Class(class) => dump_class(class, source, depth),
        Node::Module(module) => dump_module(module, source, depth),
        Node::Constant(constant) => dump_constant(constant, source, depth),
        Node::Interface(interface) => {
            println!("{indent}interface {:?}", class_name(&interface.name()));
        }
        Node::TypeAlias(alias) => {
            println!("{indent}type alias {}", class_name(&alias.name()));
        }
        Node::ClassAlias(alias) => {
            println!("{indent}class alias {}", class_name(&alias.new_name()));
        }
        Node::ModuleAlias(alias) => {
            println!("{indent}module alias {}", class_name(&alias.new_name()));
        }
        Node::MethodDefinition(method) => dump_method(method, source, depth),
        Node::AttrReader(attr) => {
            println!(
                "{indent}attr_reader {} kind={:?} type={:?} visibility={:?} comment={:?}",
                attr.name(),
                attr.kind(),
                slice(node, source),
                attr.visibility(),
                attr.comment().map(|comment| comment_string(&comment))
            );
        }
        Node::AttrWriter(attr) => {
            println!(
                "{indent}attr_writer {} kind={:?} type={:?} visibility={:?}",
                attr.name(),
                attr.kind(),
                slice(node, source),
                attr.visibility()
            );
        }
        Node::AttrAccessor(attr) => {
            println!(
                "{indent}attr_accessor {} kind={:?} type={:?} visibility={:?}",
                attr.name(),
                attr.kind(),
                slice(node, source),
                attr.visibility()
            );
        }
        Node::Include(include) => dump_mixin("include", include, source, depth),
        Node::Extend(extend) => {
            println!("{indent}extend {}", class_name(&extend.name()));
        }
        Node::Prepend(prepend) => {
            println!("{indent}prepend {}", class_name(&prepend.name()));
        }
        Node::Alias(alias) => dump_alias(alias, depth),
        Node::InstanceVariable(variable) => {
            println!(
                "{indent}ivar {} type={:?}",
                variable.name(),
                slice(&variable.type_(), source)
            );
        }
        Node::ClassVariable(variable) => {
            println!(
                "{indent}cvar {} type={:?}",
                variable.name(),
                slice(&variable.type_(), source)
            );
        }
        Node::ClassInstanceVariable(variable) => {
            println!(
                "{indent}class ivar {} type={:?}",
                variable.name(),
                slice(&variable.type_(), source)
            );
        }
        Node::Private(_) => println!("{indent}private"),
        Node::Public(_) => println!("{indent}public"),
        other => println!("{indent}other {}", node_kind(other)),
    }
}

fn dump_class(class: &ClassNode<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{indent}class {} super={:?} comment={:?}",
        class_name(&class.name()),
        class
            .super_class()
            .map(|superclass| class_name(&superclass.name())),
        class.comment().map(|comment| comment_string(&comment))
    );
    for member in class.members().iter() {
        dump_node(&member, source, depth + 1);
    }
}

fn dump_module(module: &ModuleNode<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{indent}module {} comment={:?}",
        class_name(&module.name()),
        module.comment().map(|comment| comment_string(&comment))
    );
    for member in module.members().iter() {
        dump_node(&member, source, depth + 1);
    }
}

fn dump_constant(constant: &ConstantNode<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{indent}constant {} type={:?}",
        class_name(&constant.name()),
        slice(&constant.type_(), source)
    );
}

fn dump_method(method: &MethodDefinitionNode<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    let kind = method.kind();
    let scope = if matches!(
        kind,
        MethodDefinitionKind::Singleton | MethodDefinitionKind::SingletonInstance
    ) {
        "class"
    } else {
        "instance"
    };
    println!(
        "{indent}def {scope} {} visibility={:?} overloading={} comment={:?}",
        method.name(),
        method.visibility(),
        method.overloading(),
        method.comment().map(|comment| comment_string(&comment))
    );
    for overload in method.overloads().iter() {
        let Node::MethodDefinitionOverload(overload) = overload else {
            continue;
        };
        let method_type = overload.method_type();
        println!("{indent}  overload {:?}", slice(&method_type, source));
        dump_method_type(&method_type, source, depth + 2);
    }
}

fn dump_method_type(node: &Node<'_>, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    let Node::MethodType(method_type) = node else {
        println!("{indent}not a method type: {}", node_kind(node));
        return;
    };
    let function = method_type.type_();
    let Node::FunctionType(function) = &function else {
        println!("{indent}not a function: {}", node_kind(&function));
        return;
    };
    println!(
        "{indent}required={:?}",
        list_slices(&function.required_positionals(), source)
    );
    println!(
        "{indent}optional={:?}",
        list_slices(&function.optional_positionals(), source)
    );
    println!(
        "{indent}rest={:?}",
        function.rest_positionals().map(|node| slice(&node, source))
    );
    println!(
        "{indent}trailing={:?}",
        list_slices(&function.trailing_positionals(), source)
    );
    println!(
        "{indent}required_kw={:?}",
        hash_slices(&function.required_keywords(), source)
    );
    println!(
        "{indent}optional_kw={:?}",
        hash_slices(&function.optional_keywords(), source)
    );
    println!(
        "{indent}rest_kw={:?}",
        function.rest_keywords().map(|node| slice(&node, source))
    );
    println!(
        "{indent}return={:?}",
        slice(&function.return_type(), source)
    );
    if let Some(block) = method_type.block() {
        println!("{indent}block={:?}", slice(&block.type_(), source));
    }
}

fn dump_mixin(kind: &str, include: &IncludeNode<'_>, _source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{indent}{kind} {}", class_name(&include.name()));
}

fn dump_alias(alias: &AliasNode<'_>, depth: usize) {
    let indent = "  ".repeat(depth);
    println!(
        "{indent}alias {:?} {} -> {}",
        alias.kind(),
        alias.new_name(),
        alias.old_name()
    );
}

fn class_name(name: &TypeNameNode<'_>) -> String {
    let namespace = name.namespace();
    let mut parts = Vec::new();
    for part in namespace.path().iter() {
        if let Node::Symbol(symbol) = part {
            parts.push(symbol.to_string());
        }
    }
    parts.push(name.name().to_string());
    let joined = parts.join("::");
    if namespace.absolute() {
        format!("::{joined}")
    } else {
        joined
    }
}

fn comment_string(comment: &CommentNode<'_>) -> String {
    comment.string().to_string()
}

fn slice(node: &Node<'_>, source: &str) -> String {
    let location = node.location();
    let start = usize::try_from(location.start()).unwrap_or(0);
    let end = usize::try_from(location.end()).unwrap_or(0);
    source
        .get(start..end)
        .unwrap_or("<invalid range>")
        .to_string()
}

fn list_slices(list: &ruby_rbs::node::NodeList<'_>, source: &str) -> Vec<String> {
    list.iter().map(|node| slice(&node, source)).collect()
}

fn hash_slices(hash: &ruby_rbs::node::RBSHash<'_>, source: &str) -> Vec<(String, String)> {
    hash.iter()
        .map(|(key, value)| (slice(&key, source), slice(&value, source)))
        .collect()
}

fn node_kind(node: &Node<'_>) -> &'static str {
    match node {
        Node::Class(_) => "class",
        Node::Module(_) => "module",
        Node::Constant(_) => "constant",
        Node::Interface(_) => "interface",
        Node::TypeAlias(_) => "type alias",
        Node::ClassAlias(_) => "class alias",
        Node::ModuleAlias(_) => "module alias",
        Node::MethodDefinition(_) => "method",
        Node::AttrReader(_) => "attr_reader",
        Node::AttrWriter(_) => "attr_writer",
        Node::AttrAccessor(_) => "attr_accessor",
        Node::Include(_) => "include",
        Node::Extend(_) => "extend",
        Node::Prepend(_) => "prepend",
        Node::Alias(_) => "alias",
        Node::InstanceVariable(_) => "ivar",
        Node::ClassVariable(_) => "cvar",
        Node::ClassInstanceVariable(_) => "class ivar",
        Node::Private(_) => "private",
        Node::Public(_) => "public",
        _ => "unhandled",
    }
}
