//! Sorbet `sig` and `T::Struct` support.

use std::collections::BTreeMap;
use std::sync::Mutex;

use meadow_docstring::{DocstringOptions, parse_docstring, parse_type_list_bytes};
use meadow_model::{
    AttrInfo, Docstring, ObjectId, Param, ParamKind, Registry, Scope, SourceText, Tag, TypeExpr,
};
use meadow_parser::Event;

use crate::util::{
    MethodDetails, add_method, add_tag, has_tag, simple_tag, superclass_name, typed_tag,
};
use crate::{Handled, Plugin, PluginContext};

#[derive(Default)]
struct SorbetState {
    sigs: Vec<SigRecord>,
    struct_fields: Vec<StructField>,
    const_types: Vec<ConstType>,
    ivar_types: Vec<IvarType>,
}

struct SigRecord {
    file: SourceText,
    end: u32,
    info: SigInfo,
    docstring: Docstring,
}

#[derive(Default)]
struct SigInfo {
    params: Vec<(SourceText, TypeExpr)>,
    returns: Option<TypeExpr>,
    yields: Vec<(Option<SourceText>, TypeExpr)>,
    yield_return: Option<TypeExpr>,
    abstract_: bool,
    final_: bool,
    override_: bool,
}

struct StructField {
    namespace: ObjectId,
    name: SourceText,
    types: TypeExpr,
    writer: bool,
}

struct ConstType {
    namespace: ObjectId,
    path: SourceText,
    types: TypeExpr,
}

struct IvarType {
    namespace: ObjectId,
    name: SourceText,
    types: TypeExpr,
}

/// Sorbet signatures, `T::Struct`, and `T.let` support.
pub struct Sorbet {
    state: Mutex<SorbetState>,
}

impl Default for Sorbet {
    fn default() -> Self {
        Self {
            state: Mutex::new(SorbetState::default()),
        }
    }
}

impl Plugin for Sorbet {
    fn name(&self) -> &'static str {
        "sorbet"
    }

    fn on_event(&self, event: &Event, context: &PluginContext<'_>) -> Handled {
        let mut state = self.state.lock().expect("sorbet state lock");
        match event {
            Event::Call(call) => match call.name.as_bytes() {
                b"sig" => {
                    if let Some(block) = &call.block {
                        let mut info = parse_sig(&block.source);
                        if call.arguments.iter().any(|argument| {
                            argument
                                .literal
                                .as_ref()
                                .is_some_and(|literal| literal.as_bytes() == b"final")
                        }) {
                            info.final_ = true;
                        }
                        let docstring =
                            call.docstring
                                .as_ref()
                                .map_or_else(Docstring::default, |raw| {
                                    parse_docstring(
                                        &raw.text,
                                        raw.location,
                                        &DocstringOptions::default(),
                                    )
                                    .docstring
                                });
                        state.sigs.push(SigRecord {
                            file: context.file.clone(),
                            end: event.location().end,
                            info,
                            docstring,
                        });
                    }
                }
                b"const" | b"prop" => {
                    if let Some(field) = parse_struct_field(event, context.namespace) {
                        state.struct_fields.push(field);
                    }
                }
                _ => {}
            },
            Event::ConstantWrite(event) => {
                if let Some(types) = parse_tlet(&event.value_source) {
                    state.const_types.push(ConstType {
                        namespace: context.namespace,
                        path: event.path.clone(),
                        types,
                    });
                }
            }
            Event::InstanceVariableWrite(event) => {
                if let Some(types) = parse_tlet(&event.value_source) {
                    state.ivar_types.push(IvarType {
                        namespace: context.namespace,
                        name: event.name.clone(),
                        types,
                    });
                }
            }
            _ => {}
        }
        Handled::No
    }

    fn after_handle(&self, registry: &mut Registry) {
        let state = self.state.lock().expect("sorbet state lock");
        let mut used = vec![false; state.sigs.len()];
        let mut id_files: BTreeMap<ObjectId, SourceText> = BTreeMap::new();
        for (file, ids) in registry.files() {
            for id in ids {
                id_files.insert(*id, file.clone());
            }
        }

        let methods: Vec<ObjectId> = registry
            .objects()
            .iter()
            .filter(|object| object.kind == meadow_model::ObjectKind::Method)
            .map(|object| object.id)
            .collect();

        for id in methods {
            let Some(file) = id_files.get(&id) else {
                continue;
            };
            let start = registry
                .get(id)
                .sources
                .first()
                .map_or(0, |location| location.start);
            let mut best: Option<usize> = None;
            for (index, sig) in state.sigs.iter().enumerate() {
                if used[index] || &sig.file != file || sig.end > start {
                    continue;
                }
                best = match best {
                    Some(current) if state.sigs[current].end >= sig.end => Some(current),
                    _ => Some(index),
                };
            }
            if let Some(index) = best {
                used[index] = true;
                apply_sig(
                    registry,
                    id,
                    &state.sigs[index].info,
                    &state.sigs[index].docstring,
                );
            }
        }

        for field in &state.struct_fields {
            if is_struct(registry, field.namespace) {
                add_struct_field(registry, field);
            }
        }

        for ivar in &state.ivar_types {
            let namespace_path = registry.get(ivar.namespace).path.clone();
            let mut path = namespace_path.as_bytes().to_vec();
            path.push(b'#');
            path.extend_from_slice(ivar.name.as_bytes());
            let path = SourceText::from_vec(path);
            if let Some(id) = registry.id_for_path_bytes(path.as_bytes()) {
                let mut object = registry.get(id).clone();
                if !has_tag(&object, "return", None) {
                    add_tag(&mut object, typed_tag("return", ivar.types.clone(), ""));
                }
                registry.get_mut(id).docstring = object.docstring;
            }
        }

        for constant in &state.const_types {
            let namespace_path = registry.get(constant.namespace).path.clone();
            let path = if constant.path.as_bytes().contains(&b':') {
                constant.path.clone()
            } else {
                let mut bytes = namespace_path.as_bytes().to_vec();
                if !bytes.is_empty() {
                    bytes.extend_from_slice(b"::");
                }
                bytes.extend_from_slice(constant.path.as_bytes());
                SourceText::from_vec(bytes)
            };
            if let Some(id) = registry.id_for_path_bytes(path.as_bytes()) {
                let mut object = registry.get(id).clone();
                if !has_tag(&object, "return", None) {
                    add_tag(&mut object, typed_tag("return", constant.types.clone(), ""));
                }
                registry.get_mut(id).docstring = object.docstring;
            }
        }
    }
}

fn apply_sig(registry: &mut Registry, id: ObjectId, info: &SigInfo, docstring: &Docstring) {
    let mut object = registry.get(id).clone();
    if !docstring.text.is_empty() || !docstring.tags.is_empty() {
        let mut merged = docstring.clone();
        for tag in &object.docstring.tags {
            let duplicate = merged
                .tags
                .iter()
                .any(|existing| existing.tag == tag.tag && existing.name == tag.name);
            if !duplicate {
                merged.tags.push(tag.clone());
            }
        }
        object.docstring = merged;
    }
    for (name, types) in &info.params {
        if !has_tag(&object, "param", Some(name)) {
            add_tag(
                &mut object,
                Tag {
                    tag: SourceText::from("param"),
                    name: Some(name.clone()),
                    types: Some(types.clone()),
                    types_source: None,
                    text: SourceText::default(),
                    docstring: None,
                    key: None,
                    default: None,
                },
            );
        }
    }
    if let Some(types) = &info.returns
        && !has_tag(&object, "return", None)
    {
        add_tag(&mut object, typed_tag("return", types.clone(), ""));
    }
    for (name, types) in &info.yields {
        if !has_tag(&object, "yieldparam", name.as_ref()) {
            add_tag(
                &mut object,
                Tag {
                    tag: SourceText::from("yieldparam"),
                    name: name.clone(),
                    types: Some(types.clone()),
                    types_source: None,
                    text: SourceText::default(),
                    docstring: None,
                    key: None,
                    default: None,
                },
            );
        }
    }
    if let Some(types) = &info.yield_return
        && !has_tag(&object, "yieldreturn", None)
    {
        add_tag(&mut object, typed_tag("yieldreturn", types.clone(), ""));
    }
    if info.abstract_ && !has_tag(&object, "abstract", None) {
        add_tag(&mut object, simple_tag("abstract", ""));
    }
    if info.final_ && !has_tag(&object, "final", None) {
        add_tag(&mut object, simple_tag("final", ""));
    }
    if info.override_ && !has_tag(&object, "override", None) {
        add_tag(&mut object, simple_tag("override", ""));
    }
    registry.get_mut(id).docstring = object.docstring;
}

fn parse_struct_field(event: &Event, namespace: ObjectId) -> Option<StructField> {
    let Event::Call(call) = event else {
        return None;
    };
    let name = call.arguments.first()?.literal.clone()?;
    let types = call
        .arguments
        .get(1)
        .and_then(|argument| sorbet_type(&argument.source.to_string_lossy()))?;
    let writer = call.name.as_bytes() == b"prop";
    Some(StructField {
        namespace,
        name,
        types,
        writer,
    })
}

fn parse_tlet(value: &SourceText) -> Option<TypeExpr> {
    let text = value.to_string_lossy();
    let text = text.trim();
    let rest = text.strip_prefix("T.let(")?.strip_suffix(')')?;
    let arguments = split_top_level(rest, ',');
    sorbet_type(arguments.get(1)?.trim())
}

fn is_struct(registry: &Registry, namespace: ObjectId) -> bool {
    superclass_name(registry, namespace)
        .is_some_and(|name| matches!(name.as_bytes(), b"T::Struct" | b"T::ImmutableStruct"))
}

fn add_struct_field(registry: &mut Registry, field: &StructField) {
    let return_tag = typed_tag("return", field.types.clone(), "");
    let mut docstring = Docstring::default();
    docstring.tags.push(return_tag);

    let reader_name = field.name.clone();
    add_method(
        registry,
        field.namespace,
        &reader_name,
        Scope::Instance,
        MethodDetails {
            docstring: docstring.clone(),
            attr: Some(AttrInfo {
                name: field.name.clone(),
                reader: true,
                writer: field.writer,
            }),
            ..MethodDetails::default()
        },
    );

    if field.writer {
        let mut writer_name = field.name.as_bytes().to_vec();
        writer_name.push(b'=');
        let writer_name = SourceText::from_vec(writer_name);
        let params = vec![Param {
            name: SourceText::from("value"),
            kind: ParamKind::Required,
            default: None,
        }];
        add_method(
            registry,
            field.namespace,
            &writer_name,
            Scope::Instance,
            MethodDetails {
                params,
                docstring,
                attr: Some(AttrInfo {
                    name: field.name.clone(),
                    reader: true,
                    writer: true,
                }),
                ..MethodDetails::default()
            },
        );
    }
}

fn parse_sig(source: &SourceText) -> SigInfo {
    let text = source.to_string_lossy();
    let body = match text.find('{') {
        Some(start) => {
            let end = text.rfind('}').unwrap_or(text.len());
            text[start + 1..end].to_string()
        }
        None => match text.find("do") {
            Some(start) => {
                let end = text.rfind("end").unwrap_or(text.len());
                text[start + 2..end].to_string()
            }
            None => text.into_owned(),
        },
    };
    parse_sig_body(&body)
}

fn parse_sig_body(body: &str) -> SigInfo {
    let bytes = body.as_bytes();
    let mut info = SigInfo::default();
    let mut index = 0;

    while index < bytes.len() {
        while index < bytes.len() && !is_ident_start(bytes[index]) {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && is_ident_char(bytes[index]) {
            index += 1;
        }
        let ident = &body[start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }

        let arguments = if bytes.get(index) == Some(&b'(') {
            let mut depth = 1;
            let arg_start = index + 1;
            index += 1;
            while index < bytes.len() && depth > 0 {
                match bytes[index] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                index += 1;
            }
            Some(&body[arg_start..index.saturating_sub(1)])
        } else {
            None
        };

        match ident {
            "params" => {
                if let Some(arguments) = arguments {
                    info.params = parse_name_type_pairs(arguments);
                }
            }
            "returns" => {
                if let Some(arguments) = arguments {
                    if let Some((yields, yield_return)) = parse_proc(arguments) {
                        info.yields = yields;
                        info.yield_return = yield_return;
                    } else {
                        info.returns = sorbet_type(arguments.trim());
                    }
                }
            }
            "void" => info.returns = Some(TypeExpr::Literal(SourceText::from("void"))),
            "abstract" => info.abstract_ = true,
            "final" => info.final_ = true,
            "override" => info.override_ = true,
            _ => {}
        }
    }
    info
}

fn parse_name_type_pairs(arguments: &str) -> Vec<(SourceText, TypeExpr)> {
    split_top_level(arguments, ',')
        .into_iter()
        .filter_map(|entry| {
            let (name, value) = entry.split_once(':')?;
            let name = SourceText::from_bytes(trim_bytes(name.as_bytes()));
            let types = sorbet_type(value)?;
            Some((name, types))
        })
        .collect()
}

type ProcSignature = (Vec<(Option<SourceText>, TypeExpr)>, Option<TypeExpr>);

fn parse_proc(text: &str) -> Option<ProcSignature> {
    let text = text.trim();
    if !text.starts_with("T.proc") {
        return None;
    }
    let params = find_call_arguments(text, "params").map_or_else(Vec::new, |arguments| {
        split_top_level(arguments, ',')
            .into_iter()
            .filter_map(|entry| {
                let (name, value) = entry.split_once(':')?;
                Some((
                    Some(SourceText::from_bytes(trim_bytes(name.as_bytes()))),
                    sorbet_type(value)?,
                ))
            })
            .collect()
    });
    let returns =
        find_call_arguments(text, "returns").and_then(|arguments| sorbet_type(arguments.trim()));
    Some((params, returns))
}

fn find_call_arguments<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}(");
    let start = text.find(&needle)? + needle.len();
    let mut depth = 1;
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() && depth > 0 {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        index += 1;
    }
    Some(&text[start..index.saturating_sub(1)])
}

fn sorbet_type(text: &str) -> Option<TypeExpr> {
    let text = text.trim();
    if let Some(inner) = strip_wrapper(text, "T.nilable") {
        let inner = sorbet_type(inner)?;
        return Some(TypeExpr::Union(vec![
            inner,
            TypeExpr::Name(SourceText::from("nil")),
        ]));
    }
    if let Some(inner) = strip_wrapper(text, "T.any") {
        return Some(TypeExpr::Union(
            split_top_level(inner, ',')
                .into_iter()
                .filter_map(|part| sorbet_type(&part))
                .collect(),
        ));
    }
    if let Some(inner) = strip_wrapper(text, "T.all") {
        return Some(TypeExpr::Union(
            split_top_level(inner, ',')
                .into_iter()
                .filter_map(|part| sorbet_type(&part))
                .collect(),
        ));
    }
    if let Some(inner) = strip_wrapper(text, "T.class_of") {
        return Some(TypeExpr::Generic {
            base: Box::new(TypeExpr::Name(SourceText::from("Class"))),
            params: vec![sorbet_type(inner)?],
        });
    }
    match text {
        "T::Boolean" => {
            return Some(TypeExpr::Union(vec![
                TypeExpr::Literal(SourceText::from("true")),
                TypeExpr::Literal(SourceText::from("false")),
            ]));
        }
        "T.untyped" => return Some(TypeExpr::Name(SourceText::from("Object"))),
        "T.noreturn" => return Some(TypeExpr::Literal(SourceText::from("void"))),
        "T.self_type" | "T.attached_class" => {
            return Some(TypeExpr::Literal(SourceText::from("self")));
        }
        _ => {}
    }
    if text.starts_with("T.type_parameter(") {
        let name = find_call_arguments(text, "type_parameter")?.trim_start_matches(':');
        return Some(TypeExpr::Name(SourceText::from_bytes(name.as_bytes())));
    }
    if text.starts_with("T.proc") {
        return Some(TypeExpr::Name(SourceText::from("Proc")));
    }
    if let Some(inner) = strip_bracket_generic(text) {
        let (name, arguments) = inner;
        let base = name.strip_prefix("T::").unwrap_or(name);
        let params: Vec<TypeExpr> = split_top_level(arguments, ',')
            .into_iter()
            .filter_map(|part| sorbet_type(&part))
            .collect();
        if base == "Hash" && params.len() == 2 {
            let mut params = params;
            let value = params.pop()?;
            let key = params.pop()?;
            return Some(TypeExpr::Hash {
                key: Box::new(key),
                value: Box::new(value),
                style: meadow_model::HashStyle::Angles,
            });
        }
        return Some(TypeExpr::Generic {
            base: Box::new(TypeExpr::Name(SourceText::from(base))),
            params,
        });
    }

    parse_type_list_bytes(text.as_bytes()).ok()
}

fn strip_wrapper<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(name)?;
    rest.strip_prefix('(')?.strip_suffix(')')
}

fn strip_bracket_generic(text: &str) -> Option<(&str, &str)> {
    let start = text.find('[')?;
    if !text.ends_with(']') {
        return None;
    }
    let name = &text[..start];
    if !name.starts_with("T::") {
        return None;
    }
    Some((name, &text[start + 1..text.len() - 1]))
}

fn split_top_level(text: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;

    for character in text.chars() {
        if let Some(active) = quote {
            current.push(character);
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                current.push(character);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(character);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(character);
            }
            character if character == separator && depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
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

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
