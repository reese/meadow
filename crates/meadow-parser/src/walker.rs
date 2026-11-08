//! Walks the Prism AST and emits the owned event stream.

use ruby_prism::{ConstantId, FullName, Location, Node, ParametersNode};

use crate::comments::CommentIndex;
use crate::event::{
    AliasEvent, Argument, BlockEvent, CallEvent, ClassCloseEvent, ClassOpenEvent,
    ClassVariableWriteEvent, ConstantWriteEvent, Event, InstanceVariableWriteEvent, MethodDefEvent,
    MethodScope, ModuleCloseEvent, ModuleOpenEvent, Param, ParamKind, SingletonClassCloseEvent,
    SingletonClassOpenEvent,
};
use crate::{SourceLocation, SourceText};

pub(crate) struct Walker {
    comments: CommentIndex,
    events: Vec<Event>,
}

impl Walker {
    pub(crate) const fn new(comments: CommentIndex) -> Self {
        Self {
            comments,
            events: Vec::new(),
        }
    }

    pub(crate) fn finish(self) -> Vec<Event> {
        let Self {
            mut events,
            comments,
        } = self;

        events.extend(comments.into_unused().into_iter().map(Event::Comment));
        events.sort_by_key(|event| event.location().start);
        events
    }

    pub(crate) fn walk_node(&mut self, node: &Node<'_>) {
        if let Some(node) = node.as_program_node() {
            self.walk_node(&node.statements().as_node());
        } else if let Some(node) = node.as_statements_node() {
            for statement in &node.body() {
                self.walk_node(&statement);
            }
        } else if !self.walk_declaration_node(node) {
            self.walk_control_flow_node(node);
        }
    }

    /// Walks declaration-like nodes, returning whether the node was handled.
    fn walk_declaration_node(&mut self, node: &Node<'_>) -> bool {
        if let Some(node) = node.as_class_node() {
            self.class_node(&node);
        } else if let Some(node) = node.as_module_node() {
            self.module_node(&node);
        } else if let Some(node) = node.as_singleton_class_node() {
            self.singleton_class_node(&node);
        } else if let Some(node) = node.as_def_node() {
            self.def_node(&node);
        } else if let Some(node) = node.as_constant_write_node() {
            let location = source_location(&node.location());
            let docstring = self.comments.take(&location);
            self.events.push(Event::ConstantWrite(ConstantWriteEvent {
                path: constant_id(&node.name()),
                value_source: Self::slice(&node.value().location()),
                docstring,
                location,
            }));
        } else if let Some(node) = node.as_constant_path_write_node() {
            let location = source_location(&node.location());
            let docstring = self.comments.take(&location);
            self.events.push(Event::ConstantWrite(ConstantWriteEvent {
                path: constant_name(&node.target().as_node()),
                value_source: Self::slice(&node.value().location()),
                docstring,
                location,
            }));
        } else if let Some(node) = node.as_instance_variable_write_node() {
            let location = source_location(&node.location());
            let docstring = self.comments.take(&location);
            self.events
                .push(Event::InstanceVariableWrite(InstanceVariableWriteEvent {
                    name: Self::slice(&node.name_loc()),
                    value_source: Self::slice(&node.value().location()),
                    docstring,
                    location,
                }));
        } else if let Some(node) = node.as_class_variable_write_node() {
            let location = source_location(&node.location());
            let docstring = self.comments.take(&location);
            self.events
                .push(Event::ClassVariableWrite(ClassVariableWriteEvent {
                    name: Self::slice(&node.name_loc()),
                    value_source: Some(Self::slice(&node.value().location())),
                    docstring,
                    location,
                }));
        } else if let Some(node) = node.as_alias_method_node() {
            let location = source_location(&node.location());
            let docstring = self.comments.take(&location);
            self.events.push(Event::Alias(AliasEvent {
                new_name: Self::alias_name(&node.new_name()),
                old_name: Self::alias_name(&node.old_name()),
                docstring,
                location,
            }));
        } else if let Some(node) = node.as_call_node() {
            self.call_node(&node);
        } else {
            return false;
        }
        true
    }

    fn walk_control_flow_node(&mut self, node: &Node<'_>) {
        if let Some(node) = node.as_if_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
            if let Some(subsequent) = node.subsequent() {
                self.walk_node(&subsequent);
            }
        } else if let Some(node) = node.as_unless_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
            if let Some(else_clause) = node.else_clause() {
                self.walk_node(&else_clause.as_node());
            }
        } else if let Some(node) = node.as_else_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_begin_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
            if let Some(rescue_clause) = node.rescue_clause() {
                self.walk_node(&rescue_clause.as_node());
            }
            if let Some(else_clause) = node.else_clause() {
                self.walk_node(&else_clause.as_node());
            }
            if let Some(ensure_clause) = node.ensure_clause() {
                self.walk_node(&ensure_clause.as_node());
            }
        } else if let Some(node) = node.as_rescue_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
            if let Some(subsequent) = node.subsequent() {
                self.walk_node(&subsequent.as_node());
            }
        } else if let Some(node) = node.as_ensure_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_case_node() {
            for condition in &node.conditions() {
                self.walk_node(&condition);
            }
            if let Some(else_clause) = node.else_clause() {
                self.walk_node(&else_clause.as_node());
            }
        } else if let Some(node) = node.as_case_match_node() {
            for condition in &node.conditions() {
                self.walk_node(&condition);
            }
            if let Some(else_clause) = node.else_clause() {
                self.walk_node(&else_clause.as_node());
            }
        } else if let Some(node) = node.as_when_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_in_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_while_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_until_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_for_node() {
            if let Some(statements) = node.statements() {
                self.walk_node(&statements.as_node());
            }
        } else if let Some(node) = node.as_parentheses_node()
            && let Some(body) = node.body()
        {
            self.walk_node(&body);
        } else if let Some(node) = node.as_shareable_constant_node() {
            self.walk_node(&node.write());
        } else if let Some(node) = node.as_lambda_node()
            && let Some(body) = node.body()
        {
            self.walk_node(&body);
        }
    }

    fn class_node(&mut self, node: &ruby_prism::ClassNode<'_>) {
        let location = source_location(&node.location());
        let docstring = self.comments.take(&location);
        let name = constant_name(&node.constant_path());
        let superclass = node.superclass().map(|node| Self::expression_name(&node));

        self.events.push(Event::ClassOpen(ClassOpenEvent {
            name: name.clone(),
            superclass,
            docstring,
            location,
        }));

        if let Some(body) = node.body() {
            self.walk_node(&body);
        }

        self.events.push(Event::ClassClose(ClassCloseEvent {
            name,
            location: source_location(&node.end_keyword_loc()),
        }));
    }

    fn module_node(&mut self, node: &ruby_prism::ModuleNode<'_>) {
        let location = source_location(&node.location());
        let docstring = self.comments.take(&location);
        let name = constant_name(&node.constant_path());

        self.events.push(Event::ModuleOpen(ModuleOpenEvent {
            name: name.clone(),
            docstring,
            location,
        }));

        if let Some(body) = node.body() {
            self.walk_node(&body);
        }

        self.events.push(Event::ModuleClose(ModuleCloseEvent {
            name,
            location: source_location(&node.end_keyword_loc()),
        }));
    }

    fn singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'_>) {
        let location = source_location(&node.location());
        let docstring = self.comments.take(&location);
        let receiver = Self::expression_name(&node.expression());

        self.events
            .push(Event::SingletonClassOpen(SingletonClassOpenEvent {
                receiver: receiver.clone(),
                docstring,
                location,
            }));

        if let Some(body) = node.body() {
            self.walk_node(&body);
        }

        self.events
            .push(Event::SingletonClassClose(SingletonClassCloseEvent {
                receiver,
                location: source_location(&node.end_keyword_loc()),
            }));
    }

    fn def_node(&mut self, node: &ruby_prism::DefNode<'_>) {
        let location = source_location(&node.location());
        let docstring = self.comments.take(&location);
        let name = constant_id(&node.name());
        let receiver = node.receiver().map(|node| Self::receiver_name(&node));
        let scope = if receiver.is_some() {
            MethodScope::Class
        } else {
            MethodScope::Instance
        };
        let parameters = node
            .parameters()
            .map_or_else(Vec::new, |params| Self::collect_params(&params));
        let signature = render_signature(&name, receiver.as_ref(), &parameters);
        let source = Self::slice(&node.location());

        self.events.push(Event::MethodDef(MethodDefEvent {
            name,
            scope,
            receiver,
            parameters,
            signature,
            source,
            docstring,
            location,
        }));

        if let Some(body) = node.body() {
            self.walk_node(&body);
        }
    }

    fn call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        let location = source_location(&node.location());
        let docstring = self.comments.take(&location);
        let receiver = node.receiver().map(|node| Self::receiver_name(&node));
        let arguments = node.arguments().map_or_else(Vec::new, |arguments| {
            arguments
                .arguments()
                .iter()
                .map(|argument| Self::argument(&argument))
                .collect()
        });
        let block_node = node.block();
        let block_body = block_node
            .as_ref()
            .and_then(Node::as_block_node)
            .and_then(|block| block.body());
        let block = block_node.and_then(|block| Self::block_event(&block));

        self.events.push(Event::Call(CallEvent {
            source: Self::slice(&node.location()),
            receiver,
            name: constant_id(&node.name()),
            arguments,
            block,
            docstring,
            location,
        }));

        if let Some(arguments) = node.arguments() {
            for argument in &arguments.arguments() {
                self.walk_node(&argument);
            }
        }

        if let Some(body) = block_body {
            self.walk_node(&body);
        }
    }

    fn collect_params(params: &ParametersNode<'_>) -> Vec<Param> {
        let mut result = Vec::new();

        for node in &params.requireds() {
            if let Some(parameter) = node.as_required_parameter_node() {
                result.push(Param {
                    name: constant_id(&parameter.name()),
                    kind: ParamKind::Required,
                    default: None,
                });
            }
        }

        for node in &params.optionals() {
            if let Some(parameter) = node.as_optional_parameter_node() {
                result.push(Param {
                    name: constant_id(&parameter.name()),
                    kind: ParamKind::Optional,
                    default: Some(Self::slice(&parameter.value().location())),
                });
            }
        }

        if let Some(rest) = params.rest() {
            result.push(Self::rest_param(&rest));
        }

        for node in &params.posts() {
            if let Some(parameter) = node.as_required_parameter_node() {
                result.push(Param {
                    name: constant_id(&parameter.name()),
                    kind: ParamKind::Post,
                    default: None,
                });
            }
        }

        for node in &params.keywords() {
            if let Some(parameter) = node.as_required_keyword_parameter_node() {
                result.push(Param {
                    name: constant_id(&parameter.name()),
                    kind: ParamKind::Keyword,
                    default: None,
                });
            } else if let Some(parameter) = node.as_optional_keyword_parameter_node() {
                result.push(Param {
                    name: constant_id(&parameter.name()),
                    kind: ParamKind::KeywordOptional,
                    default: Some(Self::slice(&parameter.value().location())),
                });
            }
        }

        if let Some(keyword_rest) = params.keyword_rest() {
            result.push(Self::keyword_rest_param(&keyword_rest));
        }

        if let Some(block) = params.block()
            && let Some(parameter) = block.as_block_parameter_node()
        {
            result.push(Param {
                name: parameter
                    .name()
                    .map_or_else(SourceText::default, |id| constant_id(&id)),
                kind: ParamKind::Block,
                default: None,
            });
        }

        result
    }

    fn rest_param(node: &Node<'_>) -> Param {
        if let Some(parameter) = node.as_rest_parameter_node() {
            Param {
                name: parameter
                    .name()
                    .map_or_else(SourceText::default, |id| constant_id(&id)),
                kind: ParamKind::Rest,
                default: None,
            }
        } else if node.as_forwarding_parameter_node().is_some() {
            Param {
                name: SourceText::default(),
                kind: ParamKind::Forwarding,
                default: None,
            }
        } else {
            Param {
                name: SourceText::default(),
                kind: ParamKind::Rest,
                default: None,
            }
        }
    }

    fn keyword_rest_param(node: &Node<'_>) -> Param {
        if let Some(parameter) = node.as_keyword_rest_parameter_node() {
            Param {
                name: parameter
                    .name()
                    .map_or_else(SourceText::default, |id| constant_id(&id)),
                kind: ParamKind::KeywordRest,
                default: None,
            }
        } else if node.as_no_keywords_parameter_node().is_some() {
            Param {
                name: SourceText::from_bytes(b"nil"),
                kind: ParamKind::KeywordRest,
                default: None,
            }
        } else if node.as_forwarding_parameter_node().is_some() {
            Param {
                name: SourceText::default(),
                kind: ParamKind::Forwarding,
                default: None,
            }
        } else {
            Param {
                name: SourceText::default(),
                kind: ParamKind::KeywordRest,
                default: None,
            }
        }
    }

    fn block_event(node: &Node<'_>) -> Option<BlockEvent> {
        let block = node.as_block_node()?;
        let parameters = block
            .parameters()
            .and_then(|node| node.as_block_parameters_node())
            .and_then(|parameters| parameters.parameters())
            .map_or_else(Vec::new, |params| Self::collect_params(&params));

        Some(BlockEvent {
            parameters,
            source: Self::slice(&block.location()),
        })
    }

    fn argument(node: &Node<'_>) -> Argument {
        let literal = node
            .as_symbol_node()
            .map(|symbol| SourceText::from_bytes(symbol.unescaped()))
            .or_else(|| {
                node.as_string_node()
                    .map(|string| SourceText::from_bytes(string.unescaped()))
            });

        Argument {
            source: Self::slice(&node.location()),
            literal,
        }
    }

    fn alias_name(node: &Node<'_>) -> SourceText {
        if let Some(symbol) = node.as_symbol_node() {
            SourceText::from_bytes(symbol.unescaped())
        } else if let Some(string) = node.as_string_node() {
            SourceText::from_bytes(string.unescaped())
        } else {
            Self::slice(&node.location())
        }
    }

    fn receiver_name(node: &Node<'_>) -> SourceText {
        if node.as_self_node().is_some() {
            SourceText::from_bytes(b"self")
        } else {
            Self::expression_name(node)
        }
    }

    fn expression_name(node: &Node<'_>) -> SourceText {
        if let Some(constant) = node.as_constant_read_node() {
            constant_id(&constant.name())
        } else if let Some(path) = node.as_constant_path_node() {
            path.full_name()
                .map_or_else(|_| Self::slice(&path.location()), SourceText::from_vec)
        } else {
            Self::slice(&node.location())
        }
    }

    fn slice(location: &Location<'_>) -> SourceText {
        SourceText::from_bytes(location.as_slice())
    }
}

fn constant_id(id: &ConstantId<'_>) -> SourceText {
    SourceText::from_bytes(id.as_slice())
}

fn constant_name(node: &Node<'_>) -> SourceText {
    if let Some(constant) = node.as_constant_read_node() {
        constant_id(&constant.name())
    } else if let Some(path) = node.as_constant_path_node() {
        path.full_name().map_or_else(
            |_| SourceText::from_bytes(path.location().as_slice()),
            SourceText::from_vec,
        )
    } else {
        SourceText::from_bytes(node.location().as_slice())
    }
}

pub(crate) fn source_location(location: &Location<'_>) -> SourceLocation {
    SourceLocation {
        start: location.start(),
        end: location.end(),
        start_line: u32::try_from(location.start_line().max(0)).unwrap_or(0),
        end_line: u32::try_from(location.end_line().max(0)).unwrap_or(0),
    }
}

fn render_signature(
    name: &SourceText,
    receiver: Option<&SourceText>,
    parameters: &[Param],
) -> SourceText {
    let mut signature = Vec::new();
    if let Some(receiver) = receiver {
        signature.extend_from_slice(receiver.as_bytes());
        signature.push(b'.');
    }
    signature.extend_from_slice(name.as_bytes());
    signature.push(b'(');

    for (index, parameter) in parameters.iter().enumerate() {
        if index > 0 {
            signature.extend_from_slice(b", ");
        }
        signature.extend_from_slice(&render_param(parameter));
    }

    signature.push(b')');
    SourceText::from_vec(signature)
}

fn render_param(parameter: &Param) -> Vec<u8> {
    let mut result = Vec::new();

    match parameter.kind {
        ParamKind::Required | ParamKind::Post => {
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Optional => {
            result.extend_from_slice(parameter.name.as_bytes());
            result.extend_from_slice(b" = ");
            if let Some(default) = &parameter.default {
                result.extend_from_slice(default.as_bytes());
            }
        }
        ParamKind::Rest => {
            result.push(b'*');
            result.extend_from_slice(parameter.name.as_bytes());
        }
        ParamKind::Keyword => {
            result.extend_from_slice(parameter.name.as_bytes());
            result.push(b':');
        }
        ParamKind::KeywordOptional => {
            result.extend_from_slice(parameter.name.as_bytes());
            result.extend_from_slice(b": ");
            if let Some(default) = &parameter.default {
                result.extend_from_slice(default.as_bytes());
            }
        }
        ParamKind::KeywordRest if parameter.name.as_bytes() == b"nil" => {
            result.extend_from_slice(b"**nil");
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
