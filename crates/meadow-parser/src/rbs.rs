//! RBS signature parsing.
//!
//! RBS files are parsed with the official `ruby-rbs` bindings and translated
//! into the same owned [`Event`] stream as Ruby source. The handler can then
//! merge type information with the objects discovered in Ruby files. Type
//! information is carried as generated YARD `@param` and `@return` tags; this
//! keeps the registry model unchanged and lets the HTML renderer display types
//! with the existing tag machinery.

use std::path::PathBuf;

use ruby_rbs::node::{
    AliasNode, AttributeKind, AttributeVisibility, ClassNode, CommentNode, ConstantNode,
    MethodDefinitionKind, MethodDefinitionNode, MethodDefinitionVisibility, ModuleNode, Node,
    SignatureNode, SymbolNode, TypeNameNode,
};

use crate::event::{
    AliasEvent, Argument, CallEvent, ClassCloseEvent, ClassOpenEvent, ClassVariableWriteEvent,
    ConstantWriteEvent, Docstring, Event, InstanceVariableWriteEvent, MethodDefEvent, MethodScope,
    ModuleCloseEvent, ModuleOpenEvent, Param, ParamKind,
};
use crate::{Diagnostic, DiagnosticLevel, FileFacts, ParseConfig, SourceLocation, SourceText};

/// Parses RBS source bytes into the common event stream.
pub(crate) fn parse_rbs_bytes(path: impl Into<PathBuf>, source: &[u8]) -> FileFacts {
    let path = path.into();
    let Ok(text) = std::str::from_utf8(source) else {
        return FileFacts {
            path,
            events: Vec::new(),
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Error,
                message: SourceText::from("RBS source must be valid UTF-8"),
                location: None,
            }],
        };
    };

    let signature = match ruby_rbs::node::parse(text) {
        Ok(signature) => signature,
        Err(message) => {
            return FileFacts {
                path,
                events: Vec::new(),
                diagnostics: vec![Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: SourceText::from_bytes(message.as_bytes()),
                    location: None,
                }],
            };
        }
    };

    let lines = LineIndex::new(text);
    let mut walker = RbsWalker {
        source: text,
        lines: &lines,
        events: Vec::new(),
        visibility: vec![SourceText::from("public")],
    };
    walker.walk_signature(&signature);

    FileFacts {
        path,
        events: walker.events,
        diagnostics: Vec::new(),
    }
}

/// Parses a UTF-8 RBS string. Kept for symmetry with the Ruby entry points.
#[must_use]
pub fn parse_rbs_source(path: impl Into<PathBuf>, source: &str) -> FileFacts {
    parse_rbs_bytes(path, source.as_bytes())
}

/// Parses RBS bytes using a [`ParseConfig`], which currently has no RBS-specific
/// settings.
pub(crate) fn parse_rbs_bytes_with(
    path: impl Into<PathBuf>,
    source: &[u8],
    _config: ParseConfig,
) -> FileFacts {
    parse_rbs_bytes(path, source)
}

struct RbsParam {
    param: Param,
    ty: Option<SourceText>,
}

#[derive(Default)]
struct RbsBlock {
    params: Vec<RbsParam>,
    return_type: Option<SourceText>,
}

#[derive(Default)]
struct RbsSignature {
    params: Vec<RbsParam>,
    block: Option<RbsBlock>,
    return_type: Option<SourceText>,
}

impl RbsSignature {
    fn plain_params(&self) -> Vec<Param> {
        let mut params: Vec<Param> = self
            .params
            .iter()
            .map(|param| {
                let mut param = param.param.clone();
                if param.name.is_empty()
                    && matches!(
                        param.kind,
                        ParamKind::Required | ParamKind::Optional | ParamKind::Post
                    )
                {
                    param.name = SourceText::from("_");
                }
                param
            })
            .collect();
        if self.block.is_some() {
            params.push(Param {
                name: SourceText::from("block"),
                kind: ParamKind::Block,
                default: None,
            });
        }
        params
    }
}

struct RbsWalker<'a> {
    source: &'a str,
    lines: &'a LineIndex,
    events: Vec<Event>,
    visibility: Vec<SourceText>,
}

impl RbsWalker<'_> {
    fn walk_signature(&mut self, signature: &SignatureNode<'_>) {
        for declaration in signature.declarations().iter() {
            self.walk_declaration(&declaration);
        }
    }

    fn walk_declaration(&mut self, node: &Node<'_>) {
        match node {
            Node::Class(class) => self.class_decl(class),
            Node::Module(module) => self.module_decl(module),
            Node::Constant(constant) => self.const_decl(constant),
            _ => {}
        }
    }

    fn class_decl(&mut self, class: &ClassNode<'_>) {
        let name = Self::type_name(&class.name());
        let docstring = self.docstring(class.comment());
        let superclass = class
            .super_class()
            .map(|superclass| Self::type_name(&superclass.name()));
        let location = self.location(&class.location());

        self.events.push(Event::ClassOpen(ClassOpenEvent {
            name: name.clone(),
            superclass,
            docstring,
            location,
        }));
        self.visibility.push(SourceText::from("public"));
        self.walk_members(&class.members());
        self.visibility.pop();
        self.events.push(Event::ClassClose(ClassCloseEvent {
            name,
            location: self.location(&class.end_location()),
        }));
    }

    fn module_decl(&mut self, module: &ModuleNode<'_>) {
        let name = Self::type_name(&module.name());
        let docstring = self.docstring(module.comment());
        let location = self.location(&module.location());

        self.events.push(Event::ModuleOpen(ModuleOpenEvent {
            name: name.clone(),
            docstring,
            location,
        }));
        self.visibility.push(SourceText::from("public"));
        self.walk_members(&module.members());
        self.visibility.pop();
        self.events.push(Event::ModuleClose(ModuleCloseEvent {
            name,
            location: self.location(&module.end_location()),
        }));
    }

    fn const_decl(&mut self, constant: &ConstantNode<'_>) {
        self.events.push(Event::ConstantWrite(ConstantWriteEvent {
            path: Self::type_name(&constant.name()),
            value_source: self.raw_text(&constant.type_()),
            docstring: self.docstring(constant.comment()),
            location: self.location(&constant.location()),
        }));
    }

    fn walk_members(&mut self, members: &ruby_rbs::node::NodeList<'_>) {
        for member in members.iter() {
            self.walk_member(&member);
        }
    }

    fn walk_member(&mut self, node: &Node<'_>) {
        match node {
            Node::Class(class) => self.class_decl(class),
            Node::Module(module) => self.module_decl(module),
            Node::Constant(constant) => self.const_decl(constant),
            Node::MethodDefinition(method) => self.method_member(method),
            Node::AttrReader(attr) => {
                let source = self.raw_text(node);
                self.attribute_member(
                    &attr.name(),
                    &attr.type_(),
                    attr.kind(),
                    attr.visibility(),
                    attr.comment(),
                    &node.location(),
                    &source,
                    true,
                    false,
                );
            }
            Node::AttrWriter(attr) => {
                let source = self.raw_text(node);
                self.attribute_member(
                    &attr.name(),
                    &attr.type_(),
                    attr.kind(),
                    attr.visibility(),
                    attr.comment(),
                    &node.location(),
                    &source,
                    false,
                    true,
                );
            }
            Node::AttrAccessor(attr) => {
                let source = self.raw_text(node);
                self.attribute_member(
                    &attr.name(),
                    &attr.type_(),
                    attr.kind(),
                    attr.visibility(),
                    attr.comment(),
                    &node.location(),
                    &source,
                    true,
                    true,
                );
            }
            Node::Include(include) => {
                self.mixin_member("include", &include.name(), &node.location());
            }
            Node::Extend(extend) => {
                self.mixin_member("extend", &extend.name(), &node.location());
            }
            Node::Prepend(prepend) => {
                self.mixin_member("prepend", &prepend.name(), &node.location());
            }
            Node::Alias(alias) => self.alias_member(alias),
            Node::InstanceVariable(variable) => {
                self.variable_member(
                    &variable.name(),
                    &variable.type_(),
                    variable.comment(),
                    &node.location(),
                    false,
                );
            }
            Node::ClassVariable(variable) => {
                self.variable_member(
                    &variable.name(),
                    &variable.type_(),
                    variable.comment(),
                    &node.location(),
                    true,
                );
            }
            Node::ClassInstanceVariable(variable) => {
                self.variable_member(
                    &variable.name(),
                    &variable.type_(),
                    variable.comment(),
                    &node.location(),
                    false,
                );
            }
            Node::Private(_) => self.visibility_member("private", &node.location()),
            Node::Public(_) => self.visibility_member("public", &node.location()),
            _ => {}
        }
    }

    fn visibility_member(&mut self, name: &str, location: &ruby_rbs::node::RBSLocationRange) {
        let name = SourceText::from(name);
        if let Some(current) = self.visibility.last_mut() {
            *current = name.clone();
        }
        self.emit_visibility(name, self.location(location));
    }

    fn method_member(&mut self, method: &MethodDefinitionNode<'_>) {
        let name = symbol_text(&method.name());
        let location = self.location(&method.location());
        let docstring = self.docstring(method.comment());
        let inline_visibility = method_visibility(method.visibility());
        let singleton = matches!(
            method.kind(),
            MethodDefinitionKind::Singleton | MethodDefinitionKind::SingletonInstance
        );
        let signatures: Vec<RbsSignature> = method
            .overloads()
            .iter()
            .filter_map(|overload| {
                let Node::MethodDefinitionOverload(overload) = overload else {
                    return None;
                };
                Some(self.method_signature(&overload.method_type()))
            })
            .collect();

        if let Some(visibility) = inline_visibility {
            self.emit_visibility(SourceText::from(visibility), location);
        }

        let params = signatures
            .first()
            .map_or_else(Vec::new, RbsSignature::plain_params);
        let docstring =
            merge_docstring_text(docstring, &Self::tags_for(&name, &signatures), location);
        self.events.push(Event::MethodDef(MethodDefEvent {
            name: name.clone(),
            scope: if singleton {
                MethodScope::Class
            } else {
                MethodScope::Instance
            },
            receiver: singleton.then(|| SourceText::from("self")),
            signature: render_signature(&name, &params),
            parameters: params,
            source: self.slice_text(&method.location()),
            docstring,
            location,
        }));

        if inline_visibility.is_some() {
            let current = self
                .visibility
                .last()
                .cloned()
                .unwrap_or_else(|| SourceText::from("public"));
            self.emit_visibility(current, location);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn attribute_member(
        &mut self,
        name: &SymbolNode<'_>,
        type_node: &Node<'_>,
        kind: AttributeKind,
        visibility: AttributeVisibility,
        comment: Option<CommentNode<'_>>,
        location: &ruby_rbs::node::RBSLocationRange,
        source: &SourceText,
        reader: bool,
        writer: bool,
    ) {
        let name = symbol_text(name);
        let ty = self.type_text(type_node);
        let location = self.location(location);
        let docstring = self.docstring(comment);
        let singleton = kind == AttributeKind::Singleton;
        let inline_visibility = attribute_visibility(visibility);

        if let Some(visibility) = inline_visibility {
            self.emit_visibility(SourceText::from(visibility), location);
        }

        if reader {
            let mut tags = Vec::new();
            push_type_tag(&mut tags, "return", &ty, None);
            let docstring = merge_docstring_text(docstring.clone(), &tags, location);
            self.events.push(Event::MethodDef(MethodDefEvent {
                name: name.clone(),
                scope: if singleton {
                    MethodScope::Class
                } else {
                    MethodScope::Instance
                },
                receiver: singleton.then(|| SourceText::from("self")),
                signature: render_signature(&name, &[]),
                parameters: Vec::new(),
                source: source.clone(),
                docstring,
                location,
            }));
        }

        if writer {
            let mut writer_name = name.as_bytes().to_vec();
            writer_name.push(b'=');
            let writer_name = SourceText::from_vec(writer_name);
            let params = vec![Param {
                name: SourceText::from("value"),
                kind: ParamKind::Required,
                default: None,
            }];
            let mut tags = Vec::new();
            push_type_tag(&mut tags, "param", &ty, Some(&SourceText::from("value")));
            let docstring = merge_docstring_text(docstring.clone(), &tags, location);
            self.events.push(Event::MethodDef(MethodDefEvent {
                name: writer_name.clone(),
                scope: if singleton {
                    MethodScope::Class
                } else {
                    MethodScope::Instance
                },
                receiver: singleton.then(|| SourceText::from("self")),
                signature: render_signature(&writer_name, &params),
                parameters: params,
                source: source.clone(),
                docstring,
                location,
            }));
        }

        if inline_visibility.is_some() {
            let current = self
                .visibility
                .last()
                .cloned()
                .unwrap_or_else(|| SourceText::from("public"));
            self.emit_visibility(current, location);
        }
    }

    fn mixin_member(
        &mut self,
        name: &'static str,
        target: &TypeNameNode<'_>,
        location: &ruby_rbs::node::RBSLocationRange,
    ) {
        let target = Self::type_name(target);
        self.events.push(Event::Call(CallEvent {
            source: SourceText::from(name),
            receiver: None,
            name: SourceText::from(name),
            arguments: vec![Argument {
                source: target.clone(),
                literal: Some(target),
            }],
            block: None,
            docstring: None,
            location: self.location(location),
        }));
    }

    fn alias_member(&mut self, alias: &AliasNode<'_>) {
        self.events.push(Event::Alias(AliasEvent {
            new_name: symbol_text(&alias.new_name()),
            old_name: symbol_text(&alias.old_name()),
            docstring: self.docstring(alias.comment()),
            location: self.location(&alias.location()),
        }));
    }

    fn variable_member(
        &mut self,
        name: &SymbolNode<'_>,
        type_node: &Node<'_>,
        comment: Option<CommentNode<'_>>,
        location: &ruby_rbs::node::RBSLocationRange,
        class_variable: bool,
    ) {
        let name = symbol_text(name);
        let value_source = self.raw_text(type_node);
        let docstring = self.docstring(comment);
        let location = self.location(location);

        if class_variable {
            self.events
                .push(Event::ClassVariableWrite(ClassVariableWriteEvent {
                    name,
                    value_source: Some(value_source),
                    docstring,
                    location,
                }));
        } else {
            self.events
                .push(Event::InstanceVariableWrite(InstanceVariableWriteEvent {
                    name,
                    value_source,
                    docstring,
                    location,
                }));
        }
    }

    fn emit_visibility(&mut self, name: SourceText, location: SourceLocation) {
        self.events.push(Event::Call(CallEvent {
            source: name.clone(),
            receiver: None,
            name,
            arguments: Vec::new(),
            block: None,
            docstring: None,
            location,
        }));
    }

    fn method_signature(&self, node: &Node<'_>) -> RbsSignature {
        let Node::MethodType(method_type) = node else {
            return RbsSignature::default();
        };
        let mut signature = self.function_signature(&method_type.type_());
        if let Some(block) = method_type.block() {
            let block = self.function_signature(&block.type_());
            signature.block = Some(RbsBlock {
                params: block.params,
                return_type: block.return_type,
            });
        }
        signature
    }

    fn function_signature(&self, node: &Node<'_>) -> RbsSignature {
        let Node::FunctionType(function) = node else {
            if let Node::UntypedFunctionType(function) = node {
                return RbsSignature {
                    params: Vec::new(),
                    block: None,
                    return_type: Some(self.type_text(&function.return_type())),
                };
            }
            return RbsSignature::default();
        };
        let mut params = Vec::new();
        for param in function.required_positionals().iter() {
            push_param(&param, ParamKind::Required, &mut params, self);
        }
        for param in function.optional_positionals().iter() {
            push_param(&param, ParamKind::Optional, &mut params, self);
        }
        if let Some(param) = function.rest_positionals() {
            push_param(&param, ParamKind::Rest, &mut params, self);
        }
        for param in function.trailing_positionals().iter() {
            push_param(&param, ParamKind::Post, &mut params, self);
        }
        for (key, value) in function.required_keywords().iter() {
            push_keyword(&key, &value, ParamKind::Keyword, &mut params, self);
        }
        for (key, value) in function.optional_keywords().iter() {
            push_keyword(&key, &value, ParamKind::KeywordOptional, &mut params, self);
        }
        if let Some(param) = function.rest_keywords() {
            push_param(&param, ParamKind::KeywordRest, &mut params, self);
        }

        RbsSignature {
            params,
            block: None,
            return_type: Some(self.type_text(&function.return_type())),
        }
    }

    fn tags_for(name: &SourceText, signatures: &[RbsSignature]) -> Vec<u8> {
        let mut tags = Vec::new();
        match signatures {
            [] => {}
            [signature] => push_signature_tags(&mut tags, signature),
            signatures => {
                for signature in signatures {
                    tags.extend_from_slice(b"@overload ");
                    tags.extend_from_slice(
                        render_signature(name, &signature.plain_params()).as_bytes(),
                    );
                    tags.push(b'\n');
                    let mut nested = Vec::new();
                    push_signature_tags(&mut nested, signature);
                    push_indented(&mut tags, &nested);
                }
            }
        }
        tags
    }

    fn docstring(&self, comment: Option<CommentNode<'_>>) -> Option<Docstring> {
        let comment = comment?;
        let location = self.location(&comment.location());
        let text = trim_comment(comment.string().as_bytes());
        if text.is_empty() {
            return None;
        }
        Some(Docstring {
            text: SourceText::from_vec(text),
            location,
        })
    }

    fn type_name(name: &TypeNameNode<'_>) -> SourceText {
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
            SourceText::from_bytes(format!("::{joined}").as_bytes())
        } else {
            SourceText::from_bytes(joined.as_bytes())
        }
    }

    fn raw_text(&self, node: &Node<'_>) -> SourceText {
        let range = node.location();
        self.slice_text(&range)
    }

    fn slice_text(&self, range: &ruby_rbs::node::RBSLocationRange) -> SourceText {
        SourceText::from_bytes(self.slice(range))
    }

    fn type_text(&self, node: &Node<'_>) -> SourceText {
        SourceText::from_vec(to_yard_type(self.raw_text(node).as_bytes()))
    }

    fn slice(&self, range: &ruby_rbs::node::RBSLocationRange) -> &[u8] {
        let start = usize::try_from(range.start()).unwrap_or(0);
        let end = usize::try_from(range.end()).unwrap_or(start);
        self.source.as_bytes().get(start..end).unwrap_or_default()
    }

    fn location(&self, range: &ruby_rbs::node::RBSLocationRange) -> SourceLocation {
        let start = usize::try_from(range.start()).unwrap_or(0);
        let end = usize::try_from(range.end()).unwrap_or(start);
        SourceLocation {
            start: u32::try_from(start).unwrap_or(u32::MAX),
            end: u32::try_from(end).unwrap_or(u32::MAX),
            start_line: self.lines.line(start),
            end_line: self.lines.line(end.saturating_sub(1)),
        }
    }
}

fn push_param(node: &Node<'_>, kind: ParamKind, out: &mut Vec<RbsParam>, walker: &RbsWalker<'_>) {
    let Node::FunctionParam(param) = node else {
        return;
    };
    out.push(RbsParam {
        param: Param {
            name: param
                .name()
                .map_or_else(SourceText::default, |name| symbol_text(&name)),
            kind,
            default: None,
        },
        ty: Some(walker.type_text(&param.type_())),
    });
}

fn push_keyword(
    key: &Node<'_>,
    value: &Node<'_>,
    kind: ParamKind,
    out: &mut Vec<RbsParam>,
    walker: &RbsWalker<'_>,
) {
    let Node::FunctionParam(param) = value else {
        return;
    };
    let name = match key {
        Node::Symbol(symbol) => symbol_text(symbol),
        _ => SourceText::default(),
    };
    out.push(RbsParam {
        param: Param {
            name,
            kind,
            default: None,
        },
        ty: Some(walker.type_text(&param.type_())),
    });
}

fn push_signature_tags(tags: &mut Vec<u8>, signature: &RbsSignature) {
    for param in &signature.params {
        if let Some(ty) = &param.ty
            && !param.param.name.is_empty()
        {
            push_type_tag(tags, "param", ty, Some(&param.param.name));
        }
    }
    if let Some(block) = &signature.block {
        for param in &block.params {
            if let Some(ty) = &param.ty
                && !param.param.name.is_empty()
            {
                push_type_tag(tags, "yieldparam", ty, Some(&param.param.name));
            }
        }
        if let Some(ty) = &block.return_type {
            push_type_tag(tags, "yieldreturn", ty, None);
        }
    }
    if let Some(ty) = &signature.return_type {
        push_type_tag(tags, "return", ty, None);
    }
}

fn push_type_tag(tags: &mut Vec<u8>, tag: &str, ty: &SourceText, name: Option<&SourceText>) {
    tags.push(b'@');
    tags.extend_from_slice(tag.as_bytes());
    tags.extend_from_slice(b" [");
    tags.extend_from_slice(ty.as_bytes());
    tags.push(b']');
    if let Some(name) = name {
        tags.push(b' ');
        tags.extend_from_slice(name.as_bytes());
    }
    tags.push(b'\n');
}

fn push_indented(tags: &mut Vec<u8>, nested: &[u8]) {
    for line in nested.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        tags.extend_from_slice(b"  ");
        tags.extend_from_slice(line);
        tags.push(b'\n');
    }
}

fn merge_docstring_text(
    docstring: Option<Docstring>,
    tags: &[u8],
    location: SourceLocation,
) -> Option<Docstring> {
    if tags.is_empty() {
        return docstring;
    }
    let mut text = docstring.map_or_else(Vec::new, |docstring| docstring.text.as_bytes().to_vec());
    if !text.is_empty() {
        text.push(b'\n');
    }
    text.extend_from_slice(tags);
    Some(Docstring {
        text: SourceText::from_vec(text),
        location,
    })
}

fn trim_comment(raw: &[u8]) -> Vec<u8> {
    let mut end = raw.len();
    while end > 0 && matches!(raw[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    raw[..end].to_vec()
}

fn symbol_text(symbol: &SymbolNode<'_>) -> SourceText {
    SourceText::from_bytes(symbol.as_bytes())
}

fn method_visibility(visibility: MethodDefinitionVisibility) -> Option<&'static str> {
    match visibility {
        MethodDefinitionVisibility::Public => Some("public"),
        MethodDefinitionVisibility::Private => Some("private"),
        MethodDefinitionVisibility::Unspecified => None,
    }
}

fn attribute_visibility(visibility: AttributeVisibility) -> Option<&'static str> {
    match visibility {
        AttributeVisibility::Public => Some("public"),
        AttributeVisibility::Private => Some("private"),
        AttributeVisibility::Unspecified => None,
    }
}

fn to_yard_type(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut closes: Vec<u8> = Vec::new();
    let mut previous = 0_u8;

    for &byte in bytes {
        match byte {
            b'[' => {
                let generic = is_type_name_byte(previous);
                if generic {
                    output.push(b'<');
                    closes.push(b'>');
                } else {
                    output.push(b'(');
                    closes.push(b')');
                }
                previous = *output.last().unwrap_or(&b'(');
            }
            b']' => {
                let close = closes.pop().unwrap_or(b']');
                output.push(close);
                previous = close;
            }
            b'|' => {
                output.push(b',');
                previous = b',';
            }
            _ => {
                output.push(byte);
                if !byte.is_ascii_whitespace() {
                    previous = byte;
                }
            }
        }
    }

    output
}

fn is_type_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(byte, b'_' | b':' | b'?' | b'!' | b']' | b')' | b'>')
        || byte >= 0x80
}

fn render_signature(name: &SourceText, params: &[Param]) -> SourceText {
    let mut signature = Vec::new();
    signature.extend_from_slice(name.as_bytes());
    signature.push(b'(');
    for (index, parameter) in params.iter().enumerate() {
        if index > 0 {
            signature.extend_from_slice(b", ");
        }
        match parameter.kind {
            ParamKind::Optional if parameter.name.as_bytes() == b"_" => {
                signature.extend_from_slice(b"_");
            }
            ParamKind::Optional => {
                signature.extend_from_slice(parameter.name.as_bytes());
                signature.extend_from_slice(b" = ?");
            }
            ParamKind::KeywordOptional => {
                signature.extend_from_slice(parameter.name.as_bytes());
                signature.extend_from_slice(b": ?");
            }
            _ => signature.extend_from_slice(&render_param(parameter)),
        }
    }
    signature.push(b')');
    SourceText::from_vec(signature)
}

fn render_param(parameter: &Param) -> Vec<u8> {
    let mut result = Vec::new();
    match parameter.kind {
        ParamKind::Required
        | ParamKind::Post
        | ParamKind::Optional
        | ParamKind::KeywordOptional => {
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Rest => {
            result.push(b'*');
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Keyword => {
            result.extend_from_slice(parameter.name.as_bytes());
            result.push(b':');
        }
        ParamKind::KeywordRest => {
            result.extend_from_slice(b"**");
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Block => {
            result.push(b'&');
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Forwarding => result.extend_from_slice(b"..."),
    }
    result
}

struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(index + 1);
            }
        }
        Self { starts }
    }

    fn line(&self, offset: usize) -> u32 {
        let index = match self.starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX)
    }
}
