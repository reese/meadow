//! Raw event to code object handlers for meadow.
//!
//! The handler consumes the owned event stream produced by `meadow-parser` and
//! builds a [`Registry`]. Handling is order-sensitive and single-threaded per
//! file: visibility, module functions, and reopened namespaces all depend on
//! the events that came before.

#![deny(missing_docs)]

use std::path::Path;

use meadow_docstring::{DocstringOptions, DocstringWarning, parse_docstring};

use meadow_model::{
    AttrInfo, CodeObject, Docstring, KindData, MixinKind, ObjectId, ObjectKind, Param, ParamKind,
    Ref, Registry, RequireKind, RequireRecord, Scope, SourceLocation, SourceText, Visibility,
};
use meadow_parser::{
    AliasEvent, CallEvent, ClassOpenEvent, ClassVariableWriteEvent, CommentEvent,
    ConstantWriteEvent, Docstring as ParserDocstring, Event, FileFacts, InstanceVariableWriteEvent,
    MethodDefEvent, MethodScope, ModuleOpenEvent, SingletonClassOpenEvent,
};
use meadow_plugins::{Handled, Plugin, PluginContext};

/// A file/event pair with the file path in a serializable form.
struct Frame {
    namespace: ObjectId,
    singleton: bool,
    visibility: Visibility,
    module_function: bool,
}

impl Frame {
    const fn new(namespace: ObjectId, singleton: bool) -> Self {
        Self {
            namespace,
            singleton,
            visibility: Visibility::Public,
            module_function: false,
        }
    }
}

struct MethodInput {
    namespace: ObjectId,
    name: SourceText,
    scope: Scope,
    params: Vec<Param>,
    source: SourceText,
    signature: SourceText,
    docstring: Docstring,
    visibility: Visibility,
    explicit: bool,
    attr: Option<AttrInfo>,
    alias_of: Option<ObjectId>,
    dynamic: bool,
    location: SourceLocation,
}

struct NamedInput {
    namespace: ObjectId,
    kind: ObjectKind,
    name: SourceText,
    path: SourceText,
    extra: KindData,
    docstring: Docstring,
    location: SourceLocation,
}

struct DirectiveInput<'a> {
    namespace: ObjectId,
    tag: &'a meadow_model::Tag,
    sub_tags: &'a [meadow_model::Tag],
    scope: Option<Scope>,
    visibility: Option<Visibility>,
    group: Option<SourceText>,
    location: SourceLocation,
}

struct MacroDef {
    namespace: ObjectId,
    name: SourceText,
    body: SourceText,
    attached_to: Option<SourceText>,
}

struct RecordedCall {
    namespace: ObjectId,
    name: SourceText,
    source: SourceText,
    arguments: Vec<SourceText>,
    created: Vec<ObjectId>,
    location: SourceLocation,
}

/// Consumes parser events and builds a registry.
pub struct Handler {
    registry: Registry,
    frames: Vec<Frame>,
    method_ranges: Vec<u32>,
    pending_visibility: Option<Visibility>,
    warnings: Vec<DocstringWarning>,
    macros: Vec<MacroDef>,
    pending_attach: Option<SourceText>,
    calls: Vec<RecordedCall>,
    plugins: Vec<Box<dyn Plugin>>,
    file: SourceText,
    rbs: bool,
}

impl Default for Handler {
    fn default() -> Self {
        Self::new()
    }
}

impl Handler {
    /// Creates a handler with a fresh registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            frames: vec![Frame::new(ObjectId::ROOT, false)],
            method_ranges: Vec::new(),
            pending_visibility: None,
            warnings: Vec::new(),
            macros: Vec::new(),
            pending_attach: None,
            calls: Vec::new(),
            plugins: Vec::new(),
            file: SourceText::default(),
            rbs: false,
        }
    }

    /// Creates a handler that extends an existing registry.
    #[must_use]
    pub fn with_registry(registry: Registry, plugins: Vec<Box<dyn Plugin>>) -> Self {
        Self {
            registry,
            frames: vec![Frame::new(ObjectId::ROOT, false)],
            method_ranges: Vec::new(),
            pending_visibility: None,
            warnings: Vec::new(),
            macros: Vec::new(),
            pending_attach: None,
            calls: Vec::new(),
            plugins,
            file: SourceText::default(),
            rbs: false,
        }
    }

    /// Creates a handler with built-in plugins enabled.
    #[must_use]
    pub fn with_plugins(plugins: Vec<Box<dyn Plugin>>) -> Self {
        Self {
            plugins,
            ..Self::new()
        }
    }

    /// Handles every event from one file.
    pub fn handle_file(&mut self, facts: &FileFacts) {
        self.file = SourceText::from_bytes(facts.path.to_string_lossy().as_bytes());
        self.rbs = facts.path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs");
        let base_depth = self.frames.len();

        for event in &facts.events {
            self.handle_event(event);
        }

        self.frames.truncate(base_depth);
        self.method_ranges.clear();
    }

    /// Returns docstring warnings collected so far.
    #[must_use]
    pub fn warnings(&self) -> &[DocstringWarning] {
        &self.warnings
    }

    /// Returns the completed registry after resolution passes.
    #[must_use]
    pub fn finish(self) -> Registry {
        self.finish_reporting().0
    }

    /// Finishes handling and returns resolution and plugin warnings.
    #[must_use]
    pub fn finish_reporting(
        mut self,
    ) -> (
        Registry,
        Vec<meadow_resolve::ResolutionWarning>,
        Vec<String>,
    ) {
        self.expand_macros();
        for plugin in &self.plugins {
            plugin.after_handle(&mut self.registry);
        }
        let resolution = meadow_resolve::resolve_registry(&mut self.registry);
        for plugin in &self.plugins {
            for object in self.registry.objects_mut() {
                plugin.transform_tags(object);
            }
        }
        let plugin_warnings = self
            .plugins
            .iter()
            .flat_map(|plugin| plugin.warnings())
            .collect();
        (self.registry, resolution, plugin_warnings)
    }

    fn handle_event(&mut self, event: &Event) {
        let start = event.location().start;
        while let Some(end) = self.method_ranges.last().copied() {
            if end <= start {
                self.method_ranges.pop();
            } else {
                break;
            }
        }

        if !self.method_ranges.is_empty()
            && !matches!(event, Event::MethodDef(_) | Event::InstanceVariableWrite(_))
        {
            return;
        }
        if !matches!(event, Event::MethodDef(_)) {
            self.pending_visibility = None;
        }

        if self.rbs {
            self.handle_rbs_event(event);
            return;
        }

        let context = PluginContext {
            file: &self.file,
            namespace: self.current_namespace(),
        };
        for plugin in &self.plugins {
            if plugin.on_event(event, &context) == Handled::Yes {
                return;
            }
        }

        match event {
            Event::ClassOpen(event) => self.class_open(event),
            Event::ClassClose(_) | Event::ModuleClose(_) | Event::SingletonClassClose(_) => {
                if self.frames.len() > 1 {
                    self.frames.pop();
                }
            }
            Event::ModuleOpen(event) => self.module_open(event),
            Event::SingletonClassOpen(event) => self.singleton_class_open(event),
            Event::MethodDef(event) => self.method_def(event),
            Event::ConstantWrite(event) => self.constant_write(event),
            Event::InstanceVariableWrite(event) => self.instance_variable_write(event),
            Event::ClassVariableWrite(event) => self.class_variable_write(event),
            Event::Alias(event) => self.alias_method(event),
            Event::Call(event) => self.call(event),
            Event::Comment(event) => self.free_comment(event),
        }
    }

    fn handle_rbs_event(&mut self, event: &Event) {
        match event {
            Event::ClassOpen(event) => self.rbs_class_open(event),
            Event::ClassClose(_) | Event::ModuleClose(_) | Event::SingletonClassClose(_) => {
                if self.frames.len() > 1 {
                    self.frames.pop();
                }
            }
            Event::ModuleOpen(event) => self.rbs_module_open(event),
            Event::SingletonClassOpen(_) | Event::Comment(_) => {}
            Event::MethodDef(event) => self.rbs_method_def(event),
            Event::ConstantWrite(event) => self.rbs_constant_write(event),
            Event::InstanceVariableWrite(event) => self.rbs_instance_variable_write(event),
            Event::ClassVariableWrite(event) => self.rbs_class_variable_write(event),
            Event::Alias(event) => self.rbs_alias(event),
            Event::Call(event) => self.rbs_call(event),
        }
    }

    fn rbs_class_open(&mut self, event: &ClassOpenEvent) {
        let path = self.qualify_path(&event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let id = self.ensure_namespace(
            ObjectKind::Class,
            &path,
            event.superclass.as_ref(),
            &docstring,
            event.location,
        );
        merge_docstring(&mut self.registry.get_mut(id).docstring, &docstring);
        self.frames.push(Frame::new(id, false));
    }

    fn rbs_module_open(&mut self, event: &ModuleOpenEvent) {
        let path = self.qualify_path(&event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let id = self.ensure_namespace(ObjectKind::Module, &path, None, &docstring, event.location);
        merge_docstring(&mut self.registry.get_mut(id).docstring, &docstring);
        self.frames.push(Frame::new(id, false));
    }

    fn rbs_method_def(&mut self, event: &MethodDefEvent) {
        let namespace = self.method_namespace(event);
        let scope = if self.current_frame().singleton {
            Scope::Class
        } else {
            match event.scope {
                MethodScope::Instance => Scope::Instance,
                MethodScope::Class => Scope::Class,
            }
        };
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let namespace_path = self.registry.get(namespace).path.clone();
        let path = join_path(&namespace_path, scope.separator(), &event.name);

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            merge_docstring(&mut self.registry.get_mut(existing).docstring, &docstring);
            self.registry.add_source(existing, event.location);
            self.registry.register_file(&self.file, existing);
            return;
        }

        self.insert_method(MethodInput {
            namespace,
            name: event.name.clone(),
            scope,
            params: convert_params(&event.parameters),
            source: event.source.clone(),
            signature: event.signature.clone(),
            docstring,
            visibility: self.current_frame().visibility,
            explicit: false,
            attr: None,
            alias_of: None,
            dynamic: true,
            location: event.location,
        });
    }

    fn rbs_constant_write(&mut self, event: &ConstantWriteEvent) {
        let path = self.qualify_path(&event.path);
        let docstring = self.docstring(event.docstring.as_ref(), None);

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            merge_docstring(&mut self.registry.get_mut(existing).docstring, &docstring);
            self.registry.add_source(existing, event.location);
            self.registry.register_file(&self.file, existing);
            return;
        }

        let (namespace, name) = self.ensure_constant_path(&path);
        self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::Constant,
            name,
            path,
            extra: KindData::Constant {
                value_source: event.value_source.clone(),
                alias_of: None,
            },
            docstring,
            location: event.location,
        });
    }

    fn rbs_instance_variable_write(&mut self, event: &InstanceVariableWriteEvent) {
        let namespace = self.current_namespace();
        let path = join_path(&self.registry.get(namespace).path, "#", &event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            merge_docstring(&mut self.registry.get_mut(existing).docstring, &docstring);
            self.registry.add_source(existing, event.location);
            self.registry.register_file(&self.file, existing);
            return;
        }

        self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::InstanceVariable,
            name: event.name.clone(),
            path,
            extra: KindData::InstanceVariable {
                value_source: Some(event.value_source.clone()),
            },
            docstring,
            location: event.location,
        });
    }

    fn rbs_class_variable_write(&mut self, event: &ClassVariableWriteEvent) {
        let namespace = self.current_namespace();
        let path = join_path(&self.registry.get(namespace).path, "#", &event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            merge_docstring(&mut self.registry.get_mut(existing).docstring, &docstring);
            self.registry.add_source(existing, event.location);
            self.registry.register_file(&self.file, existing);
            return;
        }

        self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::ClassVariable,
            name: event.name.clone(),
            path,
            extra: KindData::ClassVar {
                value_source: event.value_source.clone(),
            },
            docstring,
            location: event.location,
        });
    }

    fn rbs_alias(&mut self, event: &AliasEvent) {
        let namespace = self.current_namespace();
        let scope = self.default_scope();
        let namespace_path = self.registry.get(namespace).path.clone();
        let path = join_path(&namespace_path, scope.separator(), &event.new_name);

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            let docstring = self.docstring(event.docstring.as_ref(), None);
            merge_docstring(&mut self.registry.get_mut(existing).docstring, &docstring);
            self.registry.add_source(existing, event.location);
            self.registry.register_file(&self.file, existing);
            return;
        }

        self.create_alias(
            &event.new_name,
            &event.old_name,
            event.docstring.as_ref(),
            event.location,
        );
    }

    fn rbs_call(&mut self, event: &CallEvent) {
        match event.name.as_bytes() {
            b"include" | b"extend" | b"prepend" => self.mixin_call(event),
            b"private" | b"protected" | b"public" => self.visibility_call(event),
            _ => {}
        }
    }

    fn class_open(&mut self, event: &ClassOpenEvent) {
        let path = self.qualify_path(&event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let mut superclass = event.superclass.clone();
        let mut struct_members = Vec::new();
        if let Some(value) = &event.superclass
            && let Some(call) = parse_value_call(value)
            && call
                .receiver
                .as_ref()
                .is_some_and(|receiver| receiver.as_bytes() == b"Struct")
            && call.name.as_bytes() == b"new"
        {
            superclass = Some(SourceText::from("Struct"));
            let skip = usize::from(
                call.arguments
                    .first()
                    .and_then(|argument| argument.literal.clone())
                    .is_some()
                    && !looks_like_symbol(&call.arguments[0]),
            );
            for argument in call.arguments.iter().skip(skip) {
                if let Some(name) = argument_literal(argument) {
                    struct_members.push(name);
                }
            }
        }
        let id = self.ensure_namespace(
            ObjectKind::Class,
            &path,
            superclass.as_ref(),
            &docstring,
            event.location,
        );
        for member in struct_members {
            self.add_struct_member(id, &member, &docstring, event.location);
        }
        self.apply_directives(id, Some(id), &docstring, event.location);
        self.frames.push(Frame::new(id, false));
    }

    fn module_open(&mut self, event: &ModuleOpenEvent) {
        let path = self.qualify_path(&event.name);
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let id = self.ensure_namespace(ObjectKind::Module, &path, None, &docstring, event.location);
        self.apply_directives(id, Some(id), &docstring, event.location);
        self.frames.push(Frame::new(id, false));
    }

    fn singleton_class_open(&mut self, event: &SingletonClassOpenEvent) {
        let target = if event.receiver.as_bytes() == b"self" {
            self.current_namespace()
        } else {
            self.lookup_name(&event.receiver, self.current_namespace())
                .filter(|id| self.registry.get(*id).extra.is_namespace())
                .unwrap_or_else(|| self.current_namespace())
        };
        self.frames.push(Frame::new(target, true));
    }

    fn method_def(&mut self, event: &MethodDefEvent) {
        let namespace = self.method_namespace(event);
        let scope = if self.current_frame().singleton {
            Scope::Class
        } else {
            match event.scope {
                MethodScope::Instance => Scope::Instance,
                MethodScope::Class => Scope::Class,
            }
        };
        let params = convert_params(&event.parameters);
        let parameter_names: Vec<SourceText> = event
            .parameters
            .iter()
            .map(|param| param.name.clone())
            .collect();
        let docstring = self.docstring(event.docstring.as_ref(), Some(&parameter_names));
        let visibility = self
            .pending_visibility
            .take()
            .unwrap_or_else(|| self.current_frame().visibility);
        let id = self.insert_method(MethodInput {
            namespace,
            name: event.name.clone(),
            scope,
            params: params.clone(),
            source: event.source.clone(),
            signature: event.signature.clone(),
            docstring: docstring.clone(),
            visibility,
            explicit: true,
            attr: None,
            alias_of: None,
            dynamic: false,
            location: event.location,
        });

        if self.current_frame().module_function && scope == Scope::Instance {
            self.registry.get_mut(id).visibility = Visibility::Private;
            self.insert_method(MethodInput {
                namespace,
                name: event.name.clone(),
                scope: Scope::Class,
                params,
                source: event.source.clone(),
                signature: event.signature.clone(),
                docstring: docstring.clone(),
                visibility: Visibility::Public,
                explicit: true,
                attr: None,
                alias_of: None,
                dynamic: false,
                location: event.location,
            });
        }

        self.apply_directives(namespace, Some(id), &docstring, event.location);
        self.method_ranges.push(event.location.end);
    }

    fn constant_write(&mut self, event: &ConstantWriteEvent) {
        let path = self.qualify_path(&event.path);
        let docstring = self.docstring(event.docstring.as_ref(), None);

        if let Some(call) = parse_value_call(&event.value_source) {
            if call
                .receiver
                .as_ref()
                .is_some_and(|receiver| receiver.as_bytes() == b"Struct")
                && call.name.as_bytes() == b"new"
            {
                let struct_name = SourceText::from("Struct");
                let id = self.ensure_namespace(
                    ObjectKind::Class,
                    &path,
                    Some(&struct_name),
                    &docstring,
                    event.location,
                );
                let skip = usize::from(
                    call.arguments
                        .first()
                        .is_some_and(|argument| argument_literal(argument).is_some())
                        && !looks_like_symbol(&call.arguments[0]),
                );
                for argument in call.arguments.iter().skip(skip) {
                    if let Some(name) = argument_literal(argument) {
                        self.add_struct_member(id, &name, &docstring, event.location);
                    }
                }
                self.apply_directives(id, Some(id), &docstring, event.location);
                return;
            }
            if call
                .receiver
                .as_ref()
                .is_some_and(|receiver| receiver.as_bytes() == b"Class")
                && call.name.as_bytes() == b"new"
            {
                let superclass = call
                    .arguments
                    .first()
                    .map(|argument| argument.source.clone());
                let id = self.ensure_namespace(
                    ObjectKind::Class,
                    &path,
                    superclass.as_ref(),
                    &docstring,
                    event.location,
                );
                self.apply_directives(id, Some(id), &docstring, event.location);
                return;
            }
        }

        let (namespace, name) = self.ensure_constant_path(&path);
        let alias_of = constant_reference(&event.value_source)
            .then(|| self.resolve_ref(&event.value_source, namespace));
        let extra = KindData::Constant {
            value_source: event.value_source.clone(),
            alias_of,
        };
        let id = self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::Constant,
            name,
            path,
            extra,
            docstring: docstring.clone(),
            location: event.location,
        });
        self.apply_directives(namespace, Some(id), &docstring, event.location);
        let _ = id;
    }

    fn instance_variable_write(&mut self, event: &InstanceVariableWriteEvent) {
        if !event
            .value_source
            .to_string_lossy()
            .trim_start()
            .starts_with("T.let(")
        {
            return;
        }
        let namespace = self.current_namespace();
        let path = join_path(&self.registry.get(namespace).path, "#", &event.name);
        let extra = KindData::InstanceVariable {
            value_source: Some(event.value_source.clone()),
        };
        let docstring = self.docstring(event.docstring.as_ref(), None);
        self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::InstanceVariable,
            name: event.name.clone(),
            path,
            extra,
            docstring,
            location: event.location,
        });
    }

    fn add_struct_member(
        &mut self,
        namespace: ObjectId,
        name: &SourceText,
        docstring: &Docstring,
        location: SourceLocation,
    ) {
        let visibility =
            directive_visibility(docstring).unwrap_or_else(|| self.current_frame().visibility);
        let mut docstring = docstring.clone();
        self.expand_nested_macros(namespace, &mut docstring, location, 0);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        self.insert_method(MethodInput {
            namespace,
            name: name.clone(),
            scope: Scope::Instance,
            params: Vec::new(),
            source: SourceText::default(),
            signature: simple_signature(name, ""),
            docstring: docstring.clone(),
            visibility,
            explicit: false,
            attr: Some(AttrInfo {
                name: name.clone(),
                reader: true,
                writer: true,
            }),
            alias_of: None,
            dynamic: false,
            location,
        });
        let mut setter = name.as_bytes().to_vec();
        setter.push(b'=');
        let setter = SourceText::from_bytes(&setter);
        self.insert_method(MethodInput {
            namespace,
            name: setter.clone(),
            scope: Scope::Instance,
            params: vec![Param {
                name: SourceText::from("value"),
                kind: ParamKind::Required,
                default: None,
            }],
            source: SourceText::default(),
            signature: simple_signature(&setter, "value"),
            docstring: docstring.clone(),
            visibility,
            explicit: false,
            attr: Some(AttrInfo {
                name: name.clone(),
                reader: true,
                writer: true,
            }),
            alias_of: None,
            dynamic: false,
            location,
        });
    }

    fn class_variable_write(&mut self, event: &ClassVariableWriteEvent) {
        let namespace = self.current_namespace();
        let path = join_path(&self.registry.get(namespace).path, "#", &event.name);
        let extra = KindData::ClassVar {
            value_source: event.value_source.clone(),
        };
        let docstring = self.docstring(event.docstring.as_ref(), None);
        let id = self.insert_named(NamedInput {
            namespace,
            kind: ObjectKind::ClassVariable,
            name: event.name.clone(),
            path,
            extra,
            docstring: docstring.clone(),
            location: event.location,
        });
        self.apply_directives(namespace, Some(id), &docstring, event.location);
    }

    fn alias_method(&mut self, event: &AliasEvent) {
        self.create_alias(
            &event.new_name,
            &event.old_name,
            event.docstring.as_ref(),
            event.location,
        );
    }

    fn alias_method_call(&mut self, event: &CallEvent) {
        let Some(new_name) = event.arguments.first().map(argument_name) else {
            return;
        };
        let Some(old_name) = event.arguments.get(1).map(argument_name) else {
            return;
        };
        self.create_alias(
            &new_name,
            &old_name,
            event.docstring.as_ref(),
            event.location,
        );
    }

    fn create_alias(
        &mut self,
        new_name: &SourceText,
        old_name: &SourceText,
        docstring: Option<&ParserDocstring>,
        location: SourceLocation,
    ) {
        let namespace = self.current_namespace();
        let scope = self.default_scope();
        let target = self.find_method(namespace, old_name, scope);
        let target_data = target.map(|id| {
            let object = self.registry.get(id);
            let (params, source) = match &object.extra {
                KindData::Method { params, source, .. } => (params.clone(), source.clone()),
                _ => (Vec::new(), SourceText::default()),
            };
            (object.docstring.clone(), params, source)
        });
        let (params, source, docstring) = if let Some((target_docstring, params, source)) =
            target_data
        {
            let names: Vec<SourceText> = params.iter().map(|param| param.name.clone()).collect();
            let parsed = self.docstring(docstring, Some(&names));
            let docstring = if parsed.text.is_empty() {
                target_docstring
            } else {
                parsed
            };
            (params, source, docstring)
        } else {
            let parsed = self.docstring(docstring, None);
            (Vec::new(), SourceText::default(), parsed)
        };
        let signature = signature_for(new_name, &params);

        let id = self.insert_method(MethodInput {
            namespace,
            name: new_name.clone(),
            scope,
            params,
            source,
            signature,
            docstring,
            visibility: self.current_frame().visibility,
            explicit: false,
            attr: None,
            alias_of: target,
            dynamic: false,
            location,
        });

        if let Some(target) = target
            && let KindData::Method { aliases, .. } = &mut self.registry.get_mut(target).extra
            && !aliases.contains(&id)
        {
            aliases.push(id);
        }
    }

    fn require_call(&mut self, event: &CallEvent) {
        let kind = match event.name.as_bytes() {
            b"require" => RequireKind::Require,
            b"require_relative" => RequireKind::RequireRelative,
            _ => RequireKind::Autoload,
        };
        self.registry.add_require(RequireRecord {
            kind,
            arguments: event
                .arguments
                .iter()
                .map(|argument| argument.source.clone())
                .collect(),
            file: self.file.clone(),
            location: event.location,
        });
    }

    fn define_method_call(&mut self, event: &CallEvent) {
        let Some(name) = event
            .arguments
            .first()
            .and_then(|argument| argument.literal.clone())
        else {
            return;
        };
        let namespace = self.call_namespace(event);
        let scope = self.default_scope();
        let (params, source) = event.block.as_ref().map_or_else(
            || (Vec::new(), event.source.clone()),
            |block| (convert_params(&block.parameters), block.source.clone()),
        );
        let parameter_names: Vec<SourceText> =
            params.iter().map(|param| param.name.clone()).collect();
        let mut docstring = self.docstring(event.docstring.as_ref(), Some(&parameter_names));
        let visibility =
            directive_visibility(&docstring).unwrap_or_else(|| self.current_frame().visibility);
        self.expand_nested_macros(namespace, &mut docstring, event.location, 0);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        let signature = signature_for(&name, &params);
        self.insert_method(MethodInput {
            namespace,
            name,
            scope,
            params,
            source,
            signature,
            docstring,
            visibility,
            explicit: false,
            attr: None,
            alias_of: None,
            dynamic: true,
            location: event.location,
        });
    }

    fn call(&mut self, event: &CallEvent) {
        let namespace = self.current_namespace();
        let before = self.registry.objects().len();
        let name = event.name.as_bytes();

        match name {
            b"require" | b"require_relative" | b"autoload" => self.require_call(event),
            b"define_method" => self.define_method_call(event),
            b"private" | b"protected" | b"public" => self.visibility_call(event),
            b"private_class_method" | b"public_class_method" => self.class_visibility_call(event),
            b"private_constant" => self.private_constant(event),
            b"module_function" => self.module_function(event),
            b"include" | b"extend" | b"prepend" => self.mixin_call(event),
            b"alias_method" => self.alias_method_call(event),
            b"attr_reader" | b"attr_writer" | b"attr_accessor" | b"attr" => self.attr_call(event),
            _ => {}
        }

        if let Some(raw_docstring) = event.docstring.as_ref() {
            let docstring = self.docstring(Some(raw_docstring), None);
            self.apply_directives(namespace, None, &docstring, event.location);
        }

        let created = (before..self.registry.objects().len())
            .map(|index| ObjectId(u32::try_from(index).expect("object count exceeds u32::MAX")))
            .collect();
        self.calls.push(RecordedCall {
            namespace,
            name: event.name.clone(),
            source: event.source.clone(),
            arguments: event
                .arguments
                .iter()
                .map(|argument| argument.source.clone())
                .collect(),
            created,
            location: event.location,
        });

        if let Some(macro_name) = self.pending_attach.take()
            && let Some(definition) = self.macros.iter_mut().rev().find(|definition| {
                definition.namespace == namespace && definition.name == macro_name
            })
        {
            definition.attached_to = Some(event.name.clone());
        }
    }

    fn visibility_call(&mut self, event: &CallEvent) {
        let visibility = match event.name.as_bytes() {
            b"private" => Visibility::Private,
            b"protected" => Visibility::Protected,
            _ => Visibility::Public,
        };

        if event.arguments.is_empty() {
            self.current_frame_mut().visibility = visibility;
            return;
        }

        if event.arguments.len() == 1
            && event
                .arguments
                .iter()
                .all(|argument| argument.literal.is_none())
            && trim_start(event.arguments[0].source.as_bytes()).starts_with(b"def ")
        {
            self.pending_visibility = Some(visibility);
            return;
        }

        let namespace = self.call_namespace(event);
        for argument in &event.arguments {
            if let Some(name) = &argument.literal {
                self.set_method_visibility(namespace, name, Scope::Instance, visibility);
            }
        }
    }

    fn class_visibility_call(&mut self, event: &CallEvent) {
        let visibility = if event.name.as_bytes() == b"private_class_method" {
            Visibility::Private
        } else {
            Visibility::Public
        };
        let namespace = self.call_namespace(event);
        for argument in &event.arguments {
            if let Some(name) = &argument.literal {
                self.set_method_visibility(namespace, name, Scope::Class, visibility);
            }
        }
    }

    fn private_constant(&mut self, event: &CallEvent) {
        let namespace = self.call_namespace(event);
        let Some(name) = event
            .arguments
            .iter()
            .find_map(|argument| argument.literal.clone())
        else {
            return;
        };

        if let KindData::Namespace {
            constants_private, ..
        } = &mut self.registry.get_mut(namespace).extra
            && !constants_private.contains(&name)
        {
            constants_private.push(name);
        }
    }

    fn module_function(&mut self, event: &CallEvent) {
        if event.arguments.is_empty() {
            self.current_frame_mut().module_function = true;
            return;
        }

        let namespace = self.call_namespace(event);
        for argument in &event.arguments {
            let Some(name) = &argument.literal else {
                continue;
            };
            let Some(id) = self.find_method(namespace, name, Scope::Instance) else {
                continue;
            };
            let object = self.registry.get(id);
            let (params, source, signature, docstring) = match &object.extra {
                KindData::Method {
                    params,
                    source,
                    signature,
                    ..
                } => (
                    params.clone(),
                    source.clone(),
                    signature.clone(),
                    object.docstring.clone(),
                ),
                _ => continue,
            };
            self.registry.get_mut(id).visibility = Visibility::Private;
            self.insert_method(MethodInput {
                namespace,
                name: name.clone(),
                scope: Scope::Class,
                params,
                source,
                signature,
                docstring,
                visibility: Visibility::Public,
                explicit: true,
                attr: None,
                alias_of: None,
                dynamic: false,
                location: event.location,
            });
        }
    }

    fn mixin_call(&mut self, event: &CallEvent) {
        let kind = match event.name.as_bytes() {
            b"include" => MixinKind::Include,
            b"extend" => MixinKind::Extend,
            _ => MixinKind::Prepend,
        };
        let namespace = self.call_namespace(event);
        let Some(argument) = event.arguments.first() else {
            return;
        };
        let target = self.resolve_ref(&argument.source, namespace);
        self.registry.add_mixin(namespace, kind, target);
    }

    fn attr_call(&mut self, event: &CallEvent) {
        let namespace = self.current_namespace();
        let scope = self.default_scope();
        let mut docstring = self.docstring(event.docstring.as_ref(), None);
        let visibility =
            directive_visibility(&docstring).unwrap_or_else(|| self.current_frame().visibility);
        self.expand_nested_macros(namespace, &mut docstring, event.location, 0);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        let kind = event.name.as_bytes();
        let is_attr = kind == b"attr";

        let mut names = Vec::new();
        let mut legacy_writer = None;
        for argument in &event.arguments {
            if let Some(name) = &argument.literal {
                names.push(name.clone());
            } else if is_attr {
                match argument.source.as_bytes() {
                    b"true" => legacy_writer = Some(true),
                    b"false" => legacy_writer = Some(false),
                    _ => {}
                }
            }
        }

        for name in names {
            let (reader, writer) = if is_attr {
                (true, legacy_writer.unwrap_or(false))
            } else {
                (
                    kind == b"attr_reader" || kind == b"attr_accessor",
                    kind == b"attr_writer" || kind == b"attr_accessor",
                )
            };

            if reader {
                self.insert_method(MethodInput {
                    namespace,
                    name: name.clone(),
                    scope,
                    params: Vec::new(),
                    source: SourceText::default(),
                    signature: simple_signature(&name, ""),
                    docstring: docstring.clone(),
                    visibility,
                    explicit: false,
                    attr: Some(AttrInfo {
                        name: name.clone(),
                        reader: true,
                        writer,
                    }),
                    alias_of: None,
                    dynamic: false,
                    location: event.location,
                });
            }

            if writer {
                let mut writer_name = name.as_bytes().to_vec();
                writer_name.push(b'=');
                let writer_name = SourceText::from_vec(writer_name);
                self.insert_method(MethodInput {
                    namespace,
                    name: writer_name.clone(),
                    scope,
                    params: vec![Param {
                        name: SourceText::from_bytes(b"value"),
                        kind: ParamKind::Required,
                        default: None,
                    }],
                    source: SourceText::default(),
                    signature: simple_signature(&writer_name, "value"),
                    docstring: docstring.clone(),
                    visibility,
                    explicit: false,
                    attr: Some(AttrInfo {
                        name,
                        reader,
                        writer: true,
                    }),
                    alias_of: None,
                    dynamic: false,
                    location: event.location,
                });
            }
        }
    }

    fn ensure_namespace(
        &mut self,
        kind: ObjectKind,
        path: &SourceText,
        superclass: Option<&SourceText>,
        docstring: &Docstring,
        location: SourceLocation,
    ) -> ObjectId {
        let segments: Vec<SourceText> = path_segments(path.as_bytes())
            .into_iter()
            .map(SourceText::from_bytes)
            .collect();
        let mut parent = ObjectId::ROOT;
        let mut built = SourceText::default();
        let mut result = ObjectId::ROOT;
        let last_index = segments.len().saturating_sub(1);

        for (index, segment) in segments.iter().enumerate() {
            built = if built.is_empty() {
                segment.clone()
            } else {
                join_path(&built, "::", segment)
            };

            let is_last = index == last_index;
            let existing = self.registry.id_for_path_bytes(built.as_bytes());

            let id = if let Some(id) = existing {
                if is_last {
                    self.registry.add_source(id, location);
                    let object = self.registry.get_mut(id);
                    if object.docstring.text.is_empty() {
                        object.docstring = docstring.clone();
                    }
                    self.registry.register_file(&self.file, id);
                }
                id
            } else {
                let object_kind = if is_last { kind } else { ObjectKind::Module };
                let mut object = CodeObject::new(
                    object_kind,
                    segment.clone(),
                    built.clone(),
                    Some(parent),
                    KindData::empty_namespace(),
                );
                object.dynamic = !is_last;
                if is_last {
                    object.docstring = docstring.clone();
                    object.sources.push(location);
                }
                let id = self.registry.push(object);
                self.registry.add_child(parent, id);
                if is_last {
                    self.registry.register_file(&self.file, id);
                }
                id
            };

            if is_last {
                result = id;
                if let Some(superclass) = superclass {
                    let resolved = self.resolve_ref(superclass, id);
                    if let KindData::Namespace {
                        superclass: slot, ..
                    } = &mut self.registry.get_mut(id).extra
                        && slot.is_none()
                    {
                        *slot = Some(resolved);
                    }
                }
            }

            parent = id;
        }

        result
    }

    fn ensure_constant_path(&mut self, path: &SourceText) -> (ObjectId, SourceText) {
        let segments: Vec<&[u8]> = path_segments(path.as_bytes());
        let Some((last, parents)) = segments.split_last() else {
            return (self.current_namespace(), path.clone());
        };

        if parents.is_empty() {
            return (self.current_namespace(), SourceText::from_bytes(last));
        }

        let mut parent = ObjectId::ROOT;
        let mut built = SourceText::default();
        for segment in parents {
            built = if built.is_empty() {
                SourceText::from_bytes(segment)
            } else {
                join_path(&built, "::", &SourceText::from_bytes(segment))
            };
            parent = self.ensure_container(&built);
        }

        (parent, SourceText::from_bytes(last))
    }

    fn ensure_container(&mut self, path: &SourceText) -> ObjectId {
        let segments: Vec<SourceText> = path_segments(path.as_bytes())
            .into_iter()
            .map(SourceText::from_bytes)
            .collect();
        let mut parent = ObjectId::ROOT;
        let mut built = SourceText::default();

        for segment in segments {
            built = if built.is_empty() {
                segment.clone()
            } else {
                join_path(&built, "::", &segment)
            };

            if let Some(id) = self.registry.id_for_path_bytes(built.as_bytes()) {
                parent = id;
                continue;
            }

            let mut object = CodeObject::new(
                ObjectKind::Module,
                segment,
                built.clone(),
                Some(parent),
                KindData::empty_namespace(),
            );
            object.dynamic = true;
            let id = self.registry.push(object);
            self.registry.add_child(parent, id);
            parent = id;
        }

        parent
    }

    fn insert_named(&mut self, input: NamedInput) -> ObjectId {
        if let Some(existing) = self.registry.id_for_path_bytes(input.path.as_bytes()) {
            let object = self.registry.get_mut(existing);
            if object.docstring.text.is_empty() {
                object.docstring = input.docstring;
            }
            object.extra = input.extra;
            self.registry.add_source(existing, input.location);
            self.registry.register_file(&self.file, existing);
            return existing;
        }

        let mut object = CodeObject::new(
            input.kind,
            input.name,
            input.path,
            Some(input.namespace),
            input.extra,
        );
        object.docstring = input.docstring;
        object.sources.push(input.location);
        let id = self.registry.push(object);
        self.registry.add_child(input.namespace, id);
        self.registry.register_file(&self.file, id);
        id
    }

    fn insert_method(&mut self, input: MethodInput) -> ObjectId {
        let namespace_path = self.registry.get(input.namespace).path.clone();
        let path = join_path(&namespace_path, input.scope.separator(), &input.name);
        let extra = KindData::Method {
            scope: input.scope,
            params: input.params,
            source: input.source,
            signature: input.signature,
            explicit: input.explicit,
            aliases: Vec::new(),
            alias_of: input.alias_of,
            attr: input.attr,
        };

        if let Some(existing) = self.registry.id_for_path_bytes(path.as_bytes()) {
            let object = self.registry.get_mut(existing);
            if !input.docstring.text.is_empty() || !input.docstring.tags.is_empty() {
                object.docstring = input.docstring;
            }
            object.visibility = input.visibility;
            object.dynamic = input.dynamic;
            object.extra = extra;
            self.registry.add_source(existing, input.location);
            self.registry.register_file(&self.file, existing);
            existing
        } else {
            let mut object = CodeObject::new(
                ObjectKind::Method,
                input.name,
                path,
                Some(input.namespace),
                extra,
            );
            object.docstring = input.docstring;
            object.visibility = input.visibility;
            object.dynamic = input.dynamic;
            object.sources.push(input.location);
            let id = self.registry.push(object);
            self.registry.add_child(input.namespace, id);
            self.registry.register_file(&self.file, id);
            id
        }
    }

    fn set_method_visibility(
        &mut self,
        namespace: ObjectId,
        name: &SourceText,
        scope: Scope,
        visibility: Visibility,
    ) {
        let namespace_path = self.registry.get(namespace).path.clone();
        let path = join_path(&namespace_path, scope.separator(), name);
        if let Some(id) = self.registry.id_for_path_bytes(path.as_bytes())
            && self.registry.get(id).kind == ObjectKind::Method
        {
            self.registry.get_mut(id).visibility = visibility;
        }
    }

    fn find_method(
        &self,
        namespace: ObjectId,
        name: &SourceText,
        scope: Scope,
    ) -> Option<ObjectId> {
        let namespace_path = self.registry.get(namespace).path.clone();
        let path = join_path(&namespace_path, scope.separator(), name);
        self.registry.id_for_path_bytes(path.as_bytes())
    }

    fn method_namespace(&self, event: &MethodDefEvent) -> ObjectId {
        event.receiver.as_ref().map_or_else(
            || self.current_namespace(),
            |receiver| {
                if receiver.as_bytes() == b"self" {
                    self.current_namespace()
                } else {
                    self.lookup_name(receiver, self.current_namespace())
                        .filter(|id| self.registry.get(*id).extra.is_namespace())
                        .unwrap_or_else(|| self.current_namespace())
                }
            },
        )
    }

    fn call_namespace(&self, event: &CallEvent) -> ObjectId {
        event.receiver.as_ref().map_or_else(
            || self.current_namespace(),
            |receiver| {
                if receiver.as_bytes() == b"self" {
                    self.current_namespace()
                } else {
                    self.lookup_name(receiver, self.current_namespace())
                        .filter(|id| self.registry.get(*id).extra.is_namespace())
                        .unwrap_or_else(|| self.current_namespace())
                }
            },
        )
    }

    fn qualify_path(&self, name: &SourceText) -> SourceText {
        let bytes = name.as_bytes();
        if let Some(rest) = bytes.strip_prefix(b"::") {
            return SourceText::from_bytes(rest);
        }

        if let Some(position) = find_double_colon(bytes) {
            let first = &bytes[..position];
            if let Some(id) = self.lookup_relative(first, self.current_namespace()) {
                let base = self.registry.get(id).path.clone();
                return join_raw(&base, &bytes[position..]);
            }
        }

        let current = self.registry.get(self.current_namespace()).path.clone();
        if current.is_empty() {
            name.clone()
        } else {
            join_path(&current, "::", name)
        }
    }

    fn resolve_ref(&self, name: &SourceText, namespace: ObjectId) -> Ref {
        self.lookup_name(name, namespace).map_or_else(
            || Ref::Proxy {
                name: name.clone(),
                namespace_hint: Some(namespace),
            },
            Ref::Resolved,
        )
    }

    fn lookup_name(&self, name: &SourceText, namespace: ObjectId) -> Option<ObjectId> {
        let bytes = name.as_bytes();
        if let Some(rest) = bytes.strip_prefix(b"::") {
            return self.registry.id_for_path_bytes(rest);
        }
        self.lookup_relative(bytes, namespace)
    }

    fn lookup_relative(&self, name: &[u8], namespace: ObjectId) -> Option<ObjectId> {
        let mut current = Some(namespace);
        while let Some(id) = current {
            let prefix = self.registry.get(id).path.as_bytes();
            let candidate = concat_path(prefix, name);
            if let Some(found) = self.registry.id_for_path_bytes(&candidate) {
                return Some(found);
            }
            current = self.registry.get(id).namespace;
        }
        None
    }

    fn docstring(
        &mut self,
        docstring: Option<&ParserDocstring>,
        parameter_names: Option<&[SourceText]>,
    ) -> Docstring {
        let Some(docstring) = docstring else {
            return Docstring::default();
        };
        let options = DocstringOptions {
            warn_unknown_tags: true,
            validate_params: parameter_names.is_some(),
            parameter_names: parameter_names.map_or_else(Vec::new, <[SourceText]>::to_vec),
        };
        let parsed = parse_docstring(&docstring.text, docstring.location, &options);
        self.warnings.extend(parsed.warnings);
        parsed.docstring
    }

    fn free_comment(&mut self, event: &CommentEvent) {
        let options = DocstringOptions {
            warn_unknown_tags: true,
            ..DocstringOptions::default()
        };
        let parsed = parse_docstring(&event.text, event.location, &options);
        self.warnings.extend(parsed.warnings);
        let namespace = self.current_namespace();
        self.apply_directives(namespace, None, &parsed.docstring, event.location);
    }

    fn apply_directives(
        &mut self,
        namespace: ObjectId,
        target: Option<ObjectId>,
        docstring: &Docstring,
        location: SourceLocation,
    ) {
        let mut scope: Option<Scope> = None;
        let mut visibility: Option<Visibility> = None;
        let mut group: Option<SourceText> = None;
        let mut index = 0;

        while index < docstring.tags.len() {
            let tag = docstring.tags[index].clone();
            match tag.tag.as_bytes() {
                b"!scope" => {
                    scope = Some(if first_token(&tag.text) == b"class" {
                        Scope::Class
                    } else {
                        Scope::Instance
                    });
                    index += 1;
                }
                b"!visibility" => {
                    visibility = Some(match first_token(&tag.text) {
                        b"private" => Visibility::Private,
                        b"protected" => Visibility::Protected,
                        _ => Visibility::Public,
                    });
                    index += 1;
                }
                b"!group" => {
                    group = Some(SourceText::from_bytes(first_line(tag.text.as_bytes())));
                    index += 1;
                }
                b"!endgroup" => {
                    group = None;
                    index += 1;
                }
                b"!macro" => {
                    self.define_macro_directive(namespace, target, &tag, location);
                    index += 1;
                }
                b"!parse" => {
                    self.apply_parse(namespace, &tag.text);
                    index += 1;
                }
                b"!method" => {
                    let end = directive_end(docstring, index + 1);
                    let sub_tags = &docstring.tags[index + 1..end];
                    self.create_directive_method(DirectiveInput {
                        namespace,
                        tag: &tag,
                        sub_tags,
                        scope,
                        visibility,
                        group: group.clone(),
                        location,
                    });
                    index = end;
                }
                b"!attribute" => {
                    let end = directive_end(docstring, index + 1);
                    let sub_tags = &docstring.tags[index + 1..end];
                    self.create_directive_attribute(DirectiveInput {
                        namespace,
                        tag: &tag,
                        sub_tags,
                        scope,
                        visibility,
                        group: group.clone(),
                        location,
                    });
                    index = end;
                }
                _ => index += 1,
            }
        }

        if let Some(target) = target {
            if let Some(visibility) = visibility {
                self.registry.get_mut(target).visibility = visibility;
            }
            if let Some(group) = group {
                self.registry.get_mut(target).group = Some(group);
            }
            let mut docstring = self.registry.get(target).docstring.clone();
            self.expand_nested_tag_docstrings(namespace, &mut docstring, location, 0);
            docstring
                .tags
                .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
            self.registry.get_mut(target).docstring = docstring;
        }
    }

    fn expand_nested_tag_docstrings(
        &mut self,
        namespace: ObjectId,
        docstring: &mut Docstring,
        location: SourceLocation,
        depth: usize,
    ) {
        if depth >= 32 {
            return;
        }
        for tag in &mut docstring.tags {
            if tag.tag.as_bytes().starts_with(b"!") {
                continue;
            }
            if let Some(nested) = &mut tag.docstring {
                self.expand_nested_macros(namespace, nested, location, depth + 1);
                nested
                    .tags
                    .retain(|nested_tag| !nested_tag.tag.as_bytes().starts_with(b"!"));
            }
        }
    }

    fn define_macro_directive(
        &mut self,
        namespace: ObjectId,
        target: Option<ObjectId>,
        tag: &meadow_model::Tag,
        location: SourceLocation,
    ) {
        let (name, body, attach) = macro_parts(tag);
        if !body.is_empty() && !name.is_empty() {
            self.macros.push(MacroDef {
                namespace,
                name: name.clone(),
                body: body.clone(),
                attached_to: None,
            });
            if attach {
                self.pending_attach = Some(name.clone());
            }
        }
        if let Some(target) = target
            && !attach
        {
            if body.is_empty() {
                if let Some(definition) = self.find_macro(namespace, &name) {
                    self.apply_macro_body(namespace, target, &definition.body, location);
                }
            } else {
                self.apply_macro_body(namespace, target, &body, location);
            }
        }
    }

    fn apply_macro_body(
        &mut self,
        namespace: ObjectId,
        target: ObjectId,
        body: &SourceText,
        location: SourceLocation,
    ) {
        let mut docstring = self.expand_macro_text(namespace, body, location, 0);
        // Directives embedded in a macro body (visibility, groups, generated
        // methods, and so on) apply to the object receiving the macro.
        self.apply_directives(namespace, Some(target), &docstring, location);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        append_docstring(&mut self.registry.get_mut(target).docstring, &docstring);
    }

    fn expand_macro_text(
        &mut self,
        namespace: ObjectId,
        body: &SourceText,
        location: SourceLocation,
        depth: usize,
    ) -> Docstring {
        let options = DocstringOptions {
            warn_unknown_tags: true,
            ..DocstringOptions::default()
        };
        let parsed = parse_docstring(body, location, &options);
        self.warnings.extend(parsed.warnings);
        let mut docstring = parsed.docstring;
        if depth < 32 {
            self.expand_nested_macros(namespace, &mut docstring, location, depth);
        }
        docstring
    }

    fn expand_nested_macros(
        &mut self,
        namespace: ObjectId,
        docstring: &mut Docstring,
        location: SourceLocation,
        depth: usize,
    ) {
        let tags = std::mem::take(&mut docstring.tags);
        let mut expanded_tags = Vec::with_capacity(tags.len());
        let mut expanded_text = Vec::new();

        for mut tag in tags {
            if tag.tag.as_bytes() != b"!macro" {
                if let Some(nested) = &mut tag.docstring {
                    self.expand_nested_macros(namespace, nested, location, depth + 1);
                    nested
                        .tags
                        .retain(|nested_tag| !nested_tag.tag.as_bytes().starts_with(b"!"));
                }
                expanded_tags.push(tag);
                continue;
            }

            let (name, body, attach) = macro_parts(&tag);
            if !body.is_empty() && !name.is_empty() {
                self.macros.push(MacroDef {
                    namespace,
                    name: name.clone(),
                    body: body.clone(),
                    attached_to: None,
                });
                if !attach {
                    let nested = self.expand_macro_text(namespace, &body, location, depth + 1);
                    if !nested.text.is_empty() {
                        expanded_text.push(nested.text);
                    }
                    expanded_tags.extend(nested.tags);
                }
            } else if !name.is_empty()
                && let Some(definition) = self.find_macro(namespace, &name)
            {
                let nested =
                    self.expand_macro_text(namespace, &definition.body, location, depth + 1);
                if !nested.text.is_empty() {
                    expanded_text.push(nested.text);
                }
                expanded_tags.extend(nested.tags);
            }
        }

        docstring.tags = expanded_tags;
        for text in expanded_text {
            if docstring.text.is_empty() {
                docstring.text = text;
            } else {
                let mut combined = docstring.text.as_bytes().to_vec();
                combined.extend_from_slice(b"\n\n");
                combined.extend_from_slice(text.as_bytes());
                docstring.text = SourceText::from_vec(combined);
            }
            if docstring.summary.is_empty() {
                docstring.summary = docstring.text.clone();
            }
        }
    }

    fn apply_parse(&mut self, namespace: ObjectId, source: &SourceText) {
        let source = dedent_bytes(source.as_bytes());
        let facts = meadow_parser::parse_source_bytes("(directive)", &source);
        self.handle_events_in_namespace(&facts.events, namespace);
    }

    fn handle_events_in_namespace(&mut self, events: &[Event], namespace: ObjectId) {
        let base_depth = self.frames.len();
        let base_ranges = self.method_ranges.len();
        self.frames.push(Frame::new(namespace, false));
        for event in events {
            self.handle_event(event);
        }
        self.frames.truncate(base_depth);
        self.method_ranges.truncate(base_ranges);
    }

    fn create_directive_method(&mut self, input: DirectiveInput<'_>) {
        let DirectiveInput {
            namespace,
            tag,
            sub_tags,
            scope,
            visibility,
            group,
            location,
        } = input;
        let (signature, body) = split_first_line(tag.text.as_bytes());
        let signature = SourceText::from_bytes(trim_bytes(signature));
        let mut ruby = Vec::from(b"def ".as_slice());
        ruby.extend_from_slice(signature.as_bytes());
        ruby.extend_from_slice(b"\nend\n");
        let facts = meadow_parser::parse_source_bytes("(directive)", &ruby);
        let Some(method) = facts.events.iter().find_map(|event| match event {
            Event::MethodDef(method) => Some(method),
            _ => None,
        }) else {
            self.docstring_warning(location, format!("invalid @!method signature: {signature}"));
            return;
        };

        let mut docstring = self.parse_directive_text(&dedent_bytes(body), location);
        docstring.tags.extend_from_slice(sub_tags);
        self.expand_nested_macros(namespace, &mut docstring, location, 0);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        let method_scope = scope.unwrap_or(match method.scope {
            MethodScope::Instance => Scope::Instance,
            MethodScope::Class => Scope::Class,
        });
        let id = self.insert_method(MethodInput {
            namespace,
            name: method.name.clone(),
            scope: method_scope,
            params: convert_params(&method.parameters),
            source: method.source.clone(),
            signature: method.signature.clone(),
            docstring,
            visibility: visibility.unwrap_or_else(|| self.current_frame().visibility),
            explicit: false,
            attr: None,
            alias_of: None,
            dynamic: true,
            location,
        });
        if group.is_some() {
            self.registry.get_mut(id).group = group;
        }
    }

    fn create_directive_attribute(&mut self, input: DirectiveInput<'_>) {
        let DirectiveInput {
            namespace,
            tag,
            sub_tags,
            scope,
            visibility,
            group,
            location,
        } = input;
        let (header, body) = split_first_line(tag.text.as_bytes());
        let mut rest = trim_bytes(header);
        let mut reader = true;
        let mut writer = true;
        if rest.starts_with(b"[")
            && let Some(end) = rest.iter().position(|byte| *byte == b']')
        {
            let mode = &rest[1..end];
            reader = mode.contains(&b'r');
            writer = mode.contains(&b'w');
            rest = trim_bytes(&rest[end + 1..]);
        }
        let name_end = rest
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(rest.len());
        let name = SourceText::from_bytes(&rest[..name_end]);
        if name.is_empty() {
            return;
        }

        let mut docstring = self.parse_directive_text(&dedent_bytes(body), location);
        docstring.tags.extend_from_slice(sub_tags);
        self.expand_nested_macros(namespace, &mut docstring, location, 0);
        docstring
            .tags
            .retain(|tag| !tag.tag.as_bytes().starts_with(b"!"));
        let scope = scope.unwrap_or(Scope::Instance);
        let visibility = visibility.unwrap_or_else(|| self.current_frame().visibility);

        if reader {
            let id = self.insert_method(MethodInput {
                namespace,
                name: name.clone(),
                scope,
                params: Vec::new(),
                source: SourceText::default(),
                signature: simple_signature(&name, ""),
                docstring: docstring.clone(),
                visibility,
                explicit: false,
                attr: Some(AttrInfo {
                    name: name.clone(),
                    reader: true,
                    writer,
                }),
                alias_of: None,
                dynamic: true,
                location,
            });
            self.registry.get_mut(id).group.clone_from(&group);
        }
        if writer {
            let mut writer_name = name.as_bytes().to_vec();
            writer_name.push(b'=');
            let writer_name = SourceText::from_vec(writer_name);
            let id = self.insert_method(MethodInput {
                namespace,
                name: writer_name.clone(),
                scope,
                params: vec![Param {
                    name: SourceText::from_bytes(b"value"),
                    kind: ParamKind::Required,
                    default: None,
                }],
                source: SourceText::default(),
                signature: simple_signature(&writer_name, "value"),
                docstring,
                visibility,
                explicit: false,
                attr: Some(AttrInfo {
                    name,
                    reader,
                    writer: true,
                }),
                alias_of: None,
                dynamic: true,
                location,
            });
            self.registry.get_mut(id).group = group;
        }
    }

    fn parse_directive_text(&mut self, text: &[u8], location: SourceLocation) -> Docstring {
        let options = DocstringOptions {
            warn_unknown_tags: true,
            ..DocstringOptions::default()
        };
        let parsed = parse_docstring(&SourceText::from_bytes(text), location, &options);
        self.warnings.extend(parsed.warnings);
        parsed.docstring
    }

    fn docstring_warning(&mut self, location: SourceLocation, message: String) {
        self.warnings.push(DocstringWarning { message, location });
    }

    fn expand_macros(&mut self) {
        let calls = std::mem::take(&mut self.calls);
        for call in calls {
            let Some(definition) = self.find_macro(call.namespace, &call.name) else {
                continue;
            };
            let expanded = expand_macro_body(&definition.body, &call);
            let docstring = self.expand_macro_text(call.namespace, &expanded, call.location, 0);

            if call.created.is_empty() {
                self.apply_directives(call.namespace, None, &docstring, call.location);
            } else {
                for id in call.created {
                    let mut existing = self.registry.get(id).docstring.clone();
                    merge_docstring(&mut existing, &docstring);
                    self.registry.get_mut(id).docstring = existing;
                }
            }
        }
    }

    fn find_macro(&self, namespace: ObjectId, call_name: &SourceText) -> Option<MacroDef> {
        let mut current = Some(namespace);
        while let Some(id) = current {
            let found = self.macros.iter().rev().find(|definition| {
                definition.namespace == id
                    && (definition.attached_to.as_ref() == Some(call_name)
                        || definition.name == *call_name)
            });
            if let Some(definition) = found {
                return Some(MacroDef {
                    namespace: definition.namespace,
                    name: definition.name.clone(),
                    body: definition.body.clone(),
                    attached_to: definition.attached_to.clone(),
                });
            }
            current = self.registry.get(id).namespace;
        }
        None
    }

    fn current_frame(&self) -> &Frame {
        self.frames.last().expect("handler always has a root frame")
    }

    fn current_frame_mut(&mut self) -> &mut Frame {
        self.frames
            .last_mut()
            .expect("handler always has a root frame")
    }

    fn current_namespace(&self) -> ObjectId {
        self.current_frame().namespace
    }

    fn default_scope(&self) -> Scope {
        if self.current_frame().singleton {
            Scope::Class
        } else {
            Scope::Instance
        }
    }
}

/// Builds a registry from parsed files in deterministic path order.
#[must_use]
pub fn build_registry(files: &[FileFacts]) -> Registry {
    build_registry_with_warnings(files).0
}

/// Builds a registry and returns the docstring warnings collected while doing so.
#[must_use]
pub fn build_registry_with_warnings(files: &[FileFacts]) -> (Registry, Vec<DocstringWarning>) {
    build_registry_with_plugins_and_warnings(files, Vec::new())
}

/// Builds a registry with the given plugins enabled.
#[must_use]
pub fn build_registry_with_plugins(files: &[FileFacts], plugins: Vec<Box<dyn Plugin>>) -> Registry {
    build_registry_with_plugins_and_warnings(files, plugins).0
}

/// Builds a registry with plugins and returns docstring warnings.
#[must_use]
pub fn build_registry_with_plugins_and_warnings(
    files: &[FileFacts],
    plugins: Vec<Box<dyn Plugin>>,
) -> (Registry, Vec<DocstringWarning>) {
    let output = build_registry_full(files, plugins);
    (output.registry, output.docstring_warnings)
}

/// Full handler output including resolution and plugin warnings.
pub struct BuildOutput {
    /// Resolved registry.
    pub registry: Registry,
    /// Docstring parsing warnings.
    pub docstring_warnings: Vec<DocstringWarning>,
    /// Unresolved-name warnings from resolution.
    pub resolution_warnings: Vec<meadow_resolve::ResolutionWarning>,
    /// Warnings emitted by plugins.
    pub plugin_warnings: Vec<String>,
}

/// Handles only the given files into an existing registry.
#[must_use]
pub fn handle_into_registry(
    registry: Registry,
    files: &[FileFacts],
    plugins: Vec<Box<dyn Plugin>>,
) -> BuildOutput {
    let sorted = ordered_facts(files);

    let mut handler = Handler::with_registry(registry, plugins);
    for facts in sorted {
        handler.handle_file(facts);
    }
    let docstring_warnings = handler.warnings().to_vec();
    let (registry, resolution_warnings, plugin_warnings) = handler.finish_reporting();
    BuildOutput {
        registry,
        docstring_warnings,
        resolution_warnings,
        plugin_warnings,
    }
}

/// Builds a registry and all warning categories.
#[must_use]
pub fn build_registry_full(files: &[FileFacts], plugins: Vec<Box<dyn Plugin>>) -> BuildOutput {
    let sorted = ordered_facts(files);

    let mut handler = Handler::with_plugins(plugins);
    for facts in sorted {
        handler.handle_file(facts);
    }
    let docstring_warnings = handler.warnings().to_vec();
    let (registry, resolution_warnings, plugin_warnings) = handler.finish_reporting();
    BuildOutput {
        registry,
        docstring_warnings,
        resolution_warnings,
        plugin_warnings,
    }
}

/// Convenience wrapper for parsing and handling a list of paths.
///
/// # Errors
///
/// Propagates I/O errors from reading a file.
pub fn build_registry_from_paths(paths: &[impl AsRef<Path>]) -> std::io::Result<Registry> {
    let mut facts = Vec::with_capacity(paths.len());
    for path in paths {
        facts.push(meadow_parser::parse_file(path)?);
    }
    Ok(build_registry(&facts))
}

fn convert_params(params: &[meadow_parser::Param]) -> Vec<Param> {
    params
        .iter()
        .map(|param| Param {
            name: param.name.clone(),
            kind: match param.kind {
                meadow_parser::ParamKind::Required => ParamKind::Required,
                meadow_parser::ParamKind::Optional => ParamKind::Optional,
                meadow_parser::ParamKind::Rest => ParamKind::Rest,
                meadow_parser::ParamKind::Post => ParamKind::Post,
                meadow_parser::ParamKind::Keyword => ParamKind::Keyword,
                meadow_parser::ParamKind::KeywordOptional => ParamKind::KeywordOptional,
                meadow_parser::ParamKind::KeywordRest => ParamKind::KeywordRest,
                meadow_parser::ParamKind::Block => ParamKind::Block,
                meadow_parser::ParamKind::Forwarding => ParamKind::Forwarding,
            },
            default: param.default.clone(),
        })
        .collect()
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if matches!(first, b' ' | b'\t' | b'\r' | b'\n') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn argument_name(argument: &meadow_parser::Argument) -> SourceText {
    argument
        .literal
        .clone()
        .unwrap_or_else(|| argument.source.clone())
}

fn signature_for(name: &SourceText, params: &[Param]) -> SourceText {
    let mut signature = name.as_bytes().to_vec();
    signature.push(b'(');
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            signature.extend_from_slice(b", ");
        }
        signature.extend_from_slice(&render_param(param));
    }
    signature.push(b')');
    SourceText::from_vec(signature)
}

fn render_param(param: &Param) -> Vec<u8> {
    let mut result = Vec::new();
    match param.kind {
        ParamKind::Required | ParamKind::Post => result.extend_from_slice(param.name.as_bytes()),
        ParamKind::Optional => {
            result.extend_from_slice(param.name.as_bytes());
            result.extend_from_slice(b" = ");
            if let Some(default) = &param.default {
                result.extend_from_slice(default.as_bytes());
            }
        }
        ParamKind::Rest => {
            result.push(b'*');
            result.extend_from_slice(param.name.as_bytes());
        }
        ParamKind::Keyword => {
            result.extend_from_slice(param.name.as_bytes());
            result.push(b':');
        }
        ParamKind::KeywordOptional => {
            result.extend_from_slice(param.name.as_bytes());
            result.extend_from_slice(b": ");
            if let Some(default) = &param.default {
                result.extend_from_slice(default.as_bytes());
            }
        }
        ParamKind::KeywordRest if param.name.as_bytes() == b"nil" => {
            result.extend_from_slice(b"**nil");
        }
        ParamKind::KeywordRest => {
            result.extend_from_slice(b"**");
            result.extend_from_slice(param.name.as_bytes());
        }
        ParamKind::Block => {
            result.push(b'&');
            result.extend_from_slice(param.name.as_bytes());
        }
        ParamKind::Forwarding => result.extend_from_slice(b"..."),
    }
    result
}

fn simple_signature(name: &SourceText, arguments: &str) -> SourceText {
    let mut signature = name.as_bytes().to_vec();
    signature.push(b'(');
    signature.extend_from_slice(arguments.as_bytes());
    signature.push(b')');
    SourceText::from_vec(signature)
}

fn join_path(prefix: &SourceText, separator: &str, name: &SourceText) -> SourceText {
    let mut result = Vec::new();
    if !prefix.is_empty() {
        result.extend_from_slice(prefix.as_bytes());
        result.extend_from_slice(separator.as_bytes());
    }
    result.extend_from_slice(name.as_bytes());
    SourceText::from_vec(result)
}

fn join_raw(prefix: &SourceText, raw: &[u8]) -> SourceText {
    let mut result = Vec::with_capacity(prefix.len() + raw.len());
    result.extend_from_slice(prefix.as_bytes());
    result.extend_from_slice(raw);
    SourceText::from_vec(result)
}

fn concat_path(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(prefix.len() + name.len() + 2);
    if !prefix.is_empty() {
        result.extend_from_slice(prefix);
        result.extend_from_slice(b"::");
    }
    result.extend_from_slice(name);
    result
}

fn path_segments(path: &[u8]) -> Vec<&[u8]> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut index = 0;

    while index + 1 < path.len() {
        if path[index] == b':' && path[index + 1] == b':' {
            segments.push(&path[start..index]);
            start = index + 2;
            index += 2;
        } else {
            index += 1;
        }
    }
    segments.push(&path[start..]);
    segments
}

fn find_double_colon(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|window| window == b"::")
}

fn directive_end(docstring: &Docstring, start: usize) -> usize {
    let mut index = start;
    while index < docstring.tags.len() && !docstring.tags[index].tag.as_bytes().starts_with(b"!") {
        index += 1;
    }
    index
}

fn split_first_line(bytes: &[u8]) -> (&[u8], &[u8]) {
    match bytes.iter().position(|byte| *byte == b'\n') {
        Some(position) => (&bytes[..position], &bytes[position + 1..]),
        None => (bytes, &[]),
    }
}

fn first_line(bytes: &[u8]) -> &[u8] {
    trim_bytes(split_first_line(bytes).0)
}

fn first_token(bytes: &SourceText) -> &[u8] {
    let trimmed = trim_bytes(bytes.as_bytes());
    let end = trimmed
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

fn trim_bytes(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = bytes.split_last() {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn dedent_bytes(raw: &[u8]) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    if lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    let indent = lines
        .iter()
        .filter(|line| !line.trim_ascii().is_empty())
        .map(|line| line.iter().take_while(|byte| **byte == b' ').count())
        .min()
        .unwrap_or(0);

    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        result.extend_from_slice(&line[indent.min(line.len())..]);
    }
    result
}

fn directive_visibility(docstring: &Docstring) -> Option<Visibility> {
    docstring
        .tags
        .iter()
        .find(|tag| tag.tag.as_bytes() == b"!visibility")
        .map(|tag| match first_token(&tag.text) {
            b"private" => Visibility::Private,
            b"protected" => Visibility::Protected,
            _ => Visibility::Public,
        })
}

fn macro_parts(tag: &meadow_model::Tag) -> (SourceText, SourceText, bool) {
    let mut name = trim_bytes(tag.name.as_ref().map_or(&[][..], |name| name.as_bytes()));
    let mut attach = false;
    if name.starts_with(b"[")
        && let Some(end) = name.iter().position(|byte| *byte == b']')
    {
        attach = &name[1..end] == b"attach";
        name = trim_bytes(&name[end + 1..]);
    }
    (
        SourceText::from_bytes(name),
        SourceText::from_vec(dedent_bytes(tag.text.as_bytes())),
        attach,
    )
}

fn append_docstring(into: &mut Docstring, from: &Docstring) {
    if !from.text.is_empty() {
        if into.text.is_empty() {
            into.text = from.text.clone();
        } else {
            let mut text = into.text.as_bytes().to_vec();
            text.extend_from_slice(b"\n\n");
            text.extend_from_slice(from.text.as_bytes());
            into.text = SourceText::from_vec(text);
        }
    }
    if into.summary.is_empty() {
        into.summary = from.summary.clone();
    }
    for tag in &from.tags {
        if tag.tag.as_bytes().starts_with(b"!") {
            continue;
        }
        let duplicate = into.tags.iter().any(|existing| {
            existing.tag == tag.tag
                && existing.name == tag.name
                && (tag.tag.as_bytes() != b"overload" || existing.text == tag.text)
        });
        if !duplicate {
            into.tags.push(tag.clone());
        }
    }
}

fn expand_macro_body(body: &SourceText, call: &RecordedCall) -> SourceText {
    let input = body.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;

    while index < input.len() {
        if input[index] != b'$' {
            output.push(input[index]);
            index += 1;
            continue;
        }

        if input.get(index + 1) == Some(&b'{')
            && let Some(end) = input[index + 2..].iter().position(|byte| *byte == b'}')
        {
            let end = index + 2 + end;
            write_macro_index(&input[index + 2..end], call, &mut output);
            index = end + 1;
            continue;
        }

        match input.get(index + 1).copied() {
            Some(b'0') => output.extend_from_slice(call.name.as_bytes()),
            Some(b'*') => write_macro_args(&call.arguments, &mut output),
            Some(b'&') => output.extend_from_slice(call.source.as_bytes()),
            Some(digit @ b'1'..=b'9') => {
                let argument = usize::from(digit - b'1');
                if let Some(value) = call.arguments.get(argument) {
                    output.extend_from_slice(value.as_bytes());
                }
            }
            _ => output.push(b'$'),
        }
        index += 2;
    }

    SourceText::from_vec(output)
}

fn write_macro_index(spec: &[u8], call: &RecordedCall, output: &mut Vec<u8>) {
    let spec = trim_bytes(spec);
    if let Some(position) = spec.iter().position(|byte| *byte == b'-') {
        let start = parse_index(&spec[..position]);
        let end = parse_index(&spec[position + 1..]);
        for argument in start..=end {
            if argument > start {
                output.extend_from_slice(b", ");
            }
            if let Some(value) = call.arguments.get(argument.saturating_sub(1)) {
                output.extend_from_slice(value.as_bytes());
            }
        }
        return;
    }

    if spec.starts_with(b"-") {
        if let Some(value) = call.arguments.last() {
            output.extend_from_slice(value.as_bytes());
        }
        return;
    }

    if let Some(argument) = parse_index(spec)
        .checked_sub(1)
        .and_then(|index| call.arguments.get(index))
    {
        output.extend_from_slice(argument.as_bytes());
    }
}

fn parse_index(bytes: &[u8]) -> usize {
    let mut value = 0;
    for byte in bytes {
        if byte.is_ascii_digit() {
            value = value * 10 + usize::from(*byte - b'0');
        }
    }
    value
}

fn write_macro_args(arguments: &[SourceText], output: &mut Vec<u8>) {
    for (index, argument) in arguments.iter().enumerate() {
        if index > 0 {
            output.extend_from_slice(b", ");
        }
        output.extend_from_slice(argument.as_bytes());
    }
}

fn ordered_facts(files: &[FileFacts]) -> Vec<&FileFacts> {
    let mut sorted: Vec<&FileFacts> = files.iter().collect();
    sorted.sort_by(|left, right| {
        let left_rbs = is_rbs_path(&left.path);
        let right_rbs = is_rbs_path(&right.path);
        left_rbs
            .cmp(&right_rbs)
            .then_with(|| left.path.cmp(&right.path))
    });
    sorted
}

fn is_rbs_path(path: &Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs")
}

fn merge_docstring(into: &mut Docstring, from: &Docstring) {
    if into.text.is_empty() {
        into.text = from.text.clone();
        into.summary = from.summary.clone();
    }
    for tag in &from.tags {
        let duplicate = into.tags.iter().any(|existing| {
            existing.tag == tag.tag
                && existing.name == tag.name
                && (tag.tag.as_bytes() != b"overload" || existing.text == tag.text)
        });
        if !duplicate {
            into.tags.push(tag.clone());
        }
    }
}

struct ValueCall {
    receiver: Option<SourceText>,
    name: SourceText,
    arguments: Vec<meadow_parser::Argument>,
}

fn parse_value_call(value: &SourceText) -> Option<ValueCall> {
    let facts = meadow_parser::parse_source_bytes("(value)", value.as_bytes());
    facts.events.iter().find_map(|event| match event {
        Event::Call(call) => Some(ValueCall {
            receiver: call.receiver.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        }),
        _ => None,
    })
}

fn argument_literal(argument: &meadow_parser::Argument) -> Option<SourceText> {
    argument.literal.clone()
}

fn looks_like_symbol(argument: &meadow_parser::Argument) -> bool {
    argument.source.as_bytes().starts_with(b":")
}

fn constant_reference(value: &SourceText) -> bool {
    let bytes = value.as_bytes().trim_ascii();
    if bytes.is_empty() {
        return false;
    }
    let bytes = bytes.strip_prefix(b"::").unwrap_or(bytes);
    bytes.first().is_some_and(u8::is_ascii_uppercase)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
}

#[cfg(test)]
mod tests {
    use super::{RecordedCall, build_registry, expand_macro_body};
    use meadow_model::{ObjectId, SourceLocation, SourceText};
    use meadow_parser::parse_source;

    use proptest::prelude::*;

    fn call(arguments: Vec<&str>) -> RecordedCall {
        RecordedCall {
            namespace: ObjectId::ROOT,
            name: SourceText::from("demo"),
            source: SourceText::from("demo(:a, :b)"),
            arguments: arguments.into_iter().map(SourceText::from).collect(),
            created: Vec::new(),
            location: SourceLocation {
                start: 0,
                end: 0,
                start_line: 1,
                end_line: 1,
            },
        }
    }

    #[test]
    fn expands_macros_defined_on_objects() {
        let source = r"
class Example
  # @!macro greeting
  #
  #   Says hello.
  #
  #   @param [String] name the person
  #   @return [String] a greeting
  def hello(name); end

  # @!macro greeting
  def also_hello(name); end

  # @!macro
  #   An anonymous macro body.
  def anonymous; end
end
";
        let registry = build_registry(&[parse_source("example.rb", source)]);
        let hello = registry.find("Example#hello").expect("hello method");
        assert!(
            hello
                .docstring
                .text
                .to_string_lossy()
                .contains("Says hello.")
        );
        assert_eq!(hello.docstring.tags.len(), 2);
        assert_eq!(hello.docstring.tags[0].tag, "param");
        assert_eq!(hello.docstring.tags[1].tag, "return");
        assert!(
            hello
                .docstring
                .tags
                .iter()
                .all(|tag| !tag.tag.as_bytes().starts_with(b"!"))
        );

        let also = registry
            .find("Example#also_hello")
            .expect("also_hello method");
        assert!(
            also.docstring
                .text
                .to_string_lossy()
                .contains("Says hello.")
        );
        assert_eq!(also.docstring.tags.len(), 2);

        let anonymous = registry
            .find("Example#anonymous")
            .expect("anonymous method");
        assert!(
            anonymous
                .docstring
                .text
                .to_string_lossy()
                .contains("An anonymous macro body.")
        );
    }

    #[test]
    fn expands_nested_macro_references() {
        let source = r"
# @!macro shared
#   Shared prose.
#
class Nested
  # @!macro bundle
  #
  #   Bundle prose.
  #
  #   @!macro shared
  #   @return [Object] result
  def value; end
end
";
        let registry = build_registry(&[parse_source("nested.rb", source)]);
        let value = registry.find("Nested#value").expect("value method");
        let text = value.docstring.text.to_string_lossy();
        assert!(text.contains("Bundle prose."), "{text}");
        assert!(text.contains("Shared prose."), "{text}");
        assert_eq!(value.docstring.tags.len(), 1);
        assert_eq!(value.docstring.tags[0].tag, "return");
    }

    #[test]
    fn applies_visibility_directives_to_objects_and_attributes() {
        let source = r"
# @!visibility private
class Hidden
  # @!visibility private
  CONST = 1

  # @!visibility protected
  attr_reader :value
end
";
        let registry = build_registry(&[parse_source("visibility.rb", source)]);
        assert_eq!(
            registry.find("Hidden").expect("class").visibility,
            meadow_model::Visibility::Private
        );
        assert_eq!(
            registry.find("Hidden::CONST").expect("constant").visibility,
            meadow_model::Visibility::Private
        );
        assert_eq!(
            registry.find("Hidden#value").expect("attribute").visibility,
            meadow_model::Visibility::Protected
        );
    }

    #[test]
    fn expands_macros_in_method_directives() {
        let source = r"
# @!macro shared
#   Shared prose.
#
class DirectiveMacros
  # @!method generated
  #   @!macro shared
  #   @return [String] result
end
";
        let registry = build_registry(&[parse_source("directive_macros.rb", source)]);
        let generated = registry
            .find("DirectiveMacros#generated")
            .expect("generated method");
        assert!(
            generated
                .docstring
                .text
                .to_string_lossy()
                .contains("Shared prose.")
        );
        assert_eq!(generated.docstring.tags.len(), 1);
        assert_eq!(generated.docstring.tags[0].tag, "return");
        assert!(
            generated
                .docstring
                .tags
                .iter()
                .all(|tag| !tag.tag.as_bytes().starts_with(b"!"))
        );
    }

    #[test]
    fn expands_macros_in_overload_subdocstrings() {
        let source = r"
# @!macro shared
#   Shared prose.
#
class Overloaded
  # @overload run(value)
  #   @!macro shared
  #   @param [Object] value the value
  def run(value); end
end
";
        let registry = build_registry(&[parse_source("overloaded.rb", source)]);
        let run = registry.find("Overloaded#run").expect("run method");
        let overload = run
            .docstring
            .tags
            .iter()
            .find(|tag| tag.tag == "overload")
            .expect("overload tag");
        let nested = overload.docstring.as_ref().expect("overload docstring");
        assert!(nested.text.to_string_lossy().contains("Shared prose."));
        assert_eq!(nested.tags.len(), 1);
        assert_eq!(nested.tags[0].tag, "param");
        assert!(
            nested
                .tags
                .iter()
                .all(|tag| !tag.tag.as_bytes().starts_with(b"!"))
        );
    }

    proptest! {
        #[test]
        fn proptest_macro_expansion_never_panics(
            body in r"\PC{0,160}",
            args in proptest::collection::vec(r"\PC{0,40}", 0..8),
        ) {
            let call = call(args.iter().map(String::as_str).collect());
            let _ = expand_macro_body(&SourceText::from_bytes(body.as_bytes()), &call);
        }
    }
}
