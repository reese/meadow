//! Shared helpers for built-in plugins.

use meadow_model::{
    AttrInfo, CodeObject, Docstring, KindData, ObjectId, ObjectKind, Param, ParamKind, Ref,
    Registry, Scope, SourceText, Tag, TypeExpr, Visibility,
};

/// The variable details of a generated method.
pub struct MethodDetails {
    /// Method parameters.
    pub params: Vec<Param>,
    /// Method docstring.
    pub docstring: Docstring,
    /// Method visibility.
    pub visibility: Visibility,
    /// Attribute metadata for accessor methods.
    pub attr: Option<AttrInfo>,
}

impl Default for MethodDetails {
    fn default() -> Self {
        Self {
            params: Vec::new(),
            docstring: Docstring::default(),
            visibility: Visibility::Public,
            attr: None,
        }
    }
}

/// Adds a method object unless one with the same path already exists.
pub fn add_method(
    registry: &mut Registry,
    namespace: ObjectId,
    name: &SourceText,
    scope: Scope,
    details: MethodDetails,
) -> ObjectId {
    let namespace_path = registry.get(namespace).path.clone();
    let path = join_method_path(&namespace_path, scope.separator(), name);
    if let Some(existing) = registry.id_for_path_bytes(path.as_bytes()) {
        return existing;
    }

    let signature = signature(name, &details.params);
    let extra = KindData::Method {
        scope,
        params: details.params,
        source: SourceText::default(),
        signature,
        explicit: false,
        aliases: Vec::new(),
        alias_of: None,
        attr: details.attr,
    };
    let mut object = CodeObject::new(
        ObjectKind::Method,
        name.clone(),
        path,
        Some(namespace),
        extra,
    );
    object.docstring = details.docstring;
    object.visibility = details.visibility;
    object.dynamic = true;
    let id = registry.push(object);
    registry.add_child(namespace, id);
    id
}

/// Joins a scope path and method name.
#[must_use]
pub fn join_method_path(prefix: &SourceText, separator: &str, name: &SourceText) -> SourceText {
    let mut result = Vec::with_capacity(prefix.len() + separator.len() + name.len());
    if !prefix.is_empty() {
        result.extend_from_slice(prefix.as_bytes());
        result.extend_from_slice(separator.as_bytes());
    }
    result.extend_from_slice(name.as_bytes());
    SourceText::from_vec(result)
}

/// Builds a simple signature string.
#[must_use]
pub fn signature(name: &SourceText, params: &[Param]) -> SourceText {
    let mut result = name.as_bytes().to_vec();
    result.push(b'(');
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            result.extend_from_slice(b", ");
        }
        result.extend_from_slice(&render_param(param));
    }
    result.push(b')');
    SourceText::from_vec(result)
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

/// Returns true when the object has a tag with the given name (and name field).
#[must_use]
pub fn has_tag(object: &CodeObject, tag: &str, name: Option<&SourceText>) -> bool {
    object.docstring.tags.iter().any(|candidate| {
        candidate.tag.as_bytes() == tag.as_bytes()
            && (name.is_none() || candidate.name.as_ref() == name)
    })
}

/// Adds a tag when it is not already present.
pub fn add_tag(object: &mut CodeObject, tag: Tag) {
    let duplicate = object
        .docstring
        .tags
        .iter()
        .any(|candidate| candidate.tag == tag.tag && candidate.name == tag.name);
    if !duplicate {
        object.docstring.tags.push(tag);
    }
}

/// Creates a tag with no type or name.
#[must_use]
pub fn simple_tag(name: &str, text: &str) -> Tag {
    Tag {
        tag: SourceText::from(name),
        name: None,
        types: None,
        types_source: None,
        text: SourceText::from(text),
        docstring: None,
        key: None,
        default: None,
    }
}

/// Creates a typed return-style tag.
#[must_use]
pub fn typed_tag(name: &str, types: TypeExpr, text: &str) -> Tag {
    Tag {
        tag: SourceText::from(name),
        name: None,
        types: Some(types),
        types_source: None,
        text: SourceText::from(text),
        docstring: None,
        key: None,
        default: None,
    }
}

/// Returns the declared superclass name for a namespace, if any.
#[must_use]
pub fn superclass_name(registry: &Registry, namespace: ObjectId) -> Option<SourceText> {
    let KindData::Namespace { superclass, .. } = &registry.get(namespace).extra else {
        return None;
    };
    match superclass {
        Some(Ref::Resolved(id)) => Some(registry.get(*id).path.clone()),
        Some(Ref::Proxy { name, .. }) => Some(name.clone()),
        None => None,
    }
}
