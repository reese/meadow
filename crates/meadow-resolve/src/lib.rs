//! Namespace, name, inheritance, and link resolution for meadow.

#![deny(missing_docs)]

mod links;

use std::collections::{BTreeMap, HashSet};

use meadow_model::{
    Docstring, KindData, MixinEntry, ObjectId, ObjectKind, Ref, Registry, SourceText,
};

pub use links::{ExpandedDocstring, LinkTargetResolver, expand_links, proxy_url_for};

/// A warning produced while resolving names or docstrings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionWarning {
    /// Human-readable message.
    pub message: String,
    /// Object whose reference failed, when known.
    pub object: Option<ObjectId>,
}

/// Resolves proxies, computes ancestor lists, applies docstring inheritance and
/// `(see ...)` references.
#[must_use]
pub fn resolve_registry(registry: &mut Registry) -> Vec<ResolutionWarning> {
    let mut warnings = resolve_proxies(registry);
    resolve_constant_aliases(registry, &mut warnings);
    compute_ancestors(registry);
    inherit_docstrings(registry);
    apply_see_references(registry, &mut warnings);
    warnings
}

/// Resolves a YARD-style name from a namespace, returning an object id.
#[must_use]
pub fn resolve_name(
    registry: &Registry,
    name: &SourceText,
    namespace: Option<ObjectId>,
) -> Option<ObjectId> {
    let bytes = name.as_bytes();
    if let Some(rest) = bytes.strip_prefix(b"::") {
        return registry.id_for_path_bytes(rest);
    }

    if let Some(namespace) = namespace {
        let prefix = registry.get(namespace).path.as_bytes();
        let mut candidate = Vec::with_capacity(prefix.len() + name.len() + 2);
        if !prefix.is_empty() {
            candidate.extend_from_slice(prefix);
            candidate.extend_from_slice(b"::");
        }
        candidate.extend_from_slice(bytes);
        if let Some(id) = registry.id_for_path_bytes(&candidate) {
            return Some(id);
        }
    }

    let mut current = namespace;
    while let Some(id) = current {
        let prefix = registry.get(id).path.as_bytes();
        let mut candidate = Vec::with_capacity(prefix.len() + name.len() + 2);
        if !prefix.is_empty() {
            candidate.extend_from_slice(prefix);
            candidate.extend_from_slice(b"::");
        }
        candidate.extend_from_slice(bytes);
        if let Some(found) = registry.id_for_path_bytes(&candidate) {
            return Some(found);
        }
        current = registry.get(id).namespace;
    }

    registry.id_for_path_bytes(bytes)
}

fn resolve_proxies(registry: &mut Registry) -> Vec<ResolutionWarning> {
    let mut warnings = Vec::new();
    let ids: Vec<ObjectId> = registry
        .objects()
        .iter()
        .filter(|object| object.extra.is_namespace())
        .map(|object| object.id)
        .collect();

    for id in ids {
        let (superclass, includes, extends, prepends, mixin_order, hint) = {
            let object = registry.get(id);
            let KindData::Namespace {
                superclass,
                includes,
                extends,
                prepends,
                mixin_order,
                ..
            } = &object.extra
            else {
                continue;
            };
            (
                superclass.clone(),
                includes.clone(),
                extends.clone(),
                prepends.clone(),
                mixin_order.clone(),
                object.namespace,
            )
        };

        let superclass =
            superclass.map(|reference| resolve_ref(registry, reference, hint, id, &mut warnings));
        let includes = includes
            .into_iter()
            .map(|reference| resolve_ref(registry, reference, hint, id, &mut warnings))
            .collect();
        let extends = extends
            .into_iter()
            .map(|reference| resolve_ref(registry, reference, hint, id, &mut warnings))
            .collect();
        let prepends = prepends
            .into_iter()
            .map(|reference| resolve_ref(registry, reference, hint, id, &mut warnings))
            .collect();
        let mixin_order: Vec<MixinEntry> = mixin_order
            .into_iter()
            .map(|mut entry| {
                entry.target = resolve_ref(registry, entry.target, hint, id, &mut warnings);
                entry
            })
            .collect();

        if let KindData::Namespace {
            superclass: target,
            includes: include_target,
            extends: extend_target,
            prepends: prepend_target,
            mixin_order: ordered,
            ..
        } = &mut registry.get_mut(id).extra
        {
            *target = superclass;
            *include_target = includes;
            *extend_target = extends;
            *prepend_target = prepends;
            *ordered = mixin_order;
        }
    }

    warnings
}

fn resolve_constant_aliases(registry: &mut Registry, warnings: &mut Vec<ResolutionWarning>) {
    let ids: Vec<ObjectId> = registry
        .objects()
        .iter()
        .filter(|object| object.kind == ObjectKind::Constant)
        .map(|object| object.id)
        .collect();
    for id in ids {
        let (alias, namespace) = {
            let object = registry.get(id);
            let KindData::Constant { alias_of, .. } = &object.extra else {
                continue;
            };
            (alias_of.clone(), object.namespace)
        };
        let Some(Ref::Proxy {
            name,
            namespace_hint,
        }) = alias
        else {
            continue;
        };
        let resolved = resolve_name(registry, &name, namespace_hint.or(namespace));
        if let KindData::Constant { alias_of, .. } = &mut registry.get_mut(id).extra {
            if let Some(target) = resolved {
                *alias_of = Some(Ref::Resolved(target));
            } else {
                warnings.push(ResolutionWarning {
                    message: format!("Cannot resolve constant alias {}", name.to_string_lossy()),
                    object: Some(id),
                });
            }
        }
    }
}

fn resolve_ref(
    registry: &Registry,
    reference: Ref,
    hint: Option<ObjectId>,
    owner: ObjectId,
    warnings: &mut Vec<ResolutionWarning>,
) -> Ref {
    match reference {
        Ref::Resolved(id) => Ref::Resolved(id),
        Ref::Proxy {
            name,
            namespace_hint,
        } => {
            let namespace = namespace_hint.or(hint);
            resolve_name(registry, &name, namespace).map_or_else(
                || {
                    warnings.push(ResolutionWarning {
                        message: format!("Cannot resolve constant {}", name.to_string_lossy()),
                        object: Some(owner),
                    });
                    Ref::Proxy {
                        name,
                        namespace_hint,
                    }
                },
                Ref::Resolved,
            )
        }
    }
}

fn compute_ancestors(registry: &mut Registry) {
    let ids: Vec<ObjectId> = registry
        .objects()
        .iter()
        .filter(|object| object.extra.is_namespace())
        .map(|object| object.id)
        .collect();
    let mut memo: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();

    for id in ids {
        let ancestors = ancestors_of(registry, id, &mut memo, &mut HashSet::new());
        if let KindData::Namespace {
            ancestors: slot, ..
        } = &mut registry.get_mut(id).extra
        {
            *slot = ancestors;
        }
    }
}

fn ancestors_of(
    registry: &Registry,
    id: ObjectId,
    memo: &mut BTreeMap<ObjectId, Vec<ObjectId>>,
    visiting: &mut HashSet<ObjectId>,
) -> Vec<ObjectId> {
    if let Some(cached) = memo.get(&id) {
        return cached.clone();
    }
    if !visiting.insert(id) {
        return Vec::new();
    }

    let (superclass, includes, prepends) = {
        let object = registry.get(id);
        let KindData::Namespace {
            superclass,
            includes,
            prepends,
            ..
        } = &object.extra
        else {
            return Vec::new();
        };
        (superclass.clone(), includes.clone(), prepends.clone())
    };

    let mut result = Vec::new();
    for reference in prepends.iter().rev() {
        if let Ref::Resolved(target) = reference {
            push_unique(&mut result, *target);
            for ancestor in ancestors_of(registry, *target, memo, visiting) {
                push_unique(&mut result, ancestor);
            }
        }
    }
    for reference in includes.iter().rev() {
        if let Ref::Resolved(target) = reference {
            push_unique(&mut result, *target);
            for ancestor in ancestors_of(registry, *target, memo, visiting) {
                push_unique(&mut result, ancestor);
            }
        }
    }
    if let Some(Ref::Resolved(target)) = superclass {
        push_unique(&mut result, target);
        for ancestor in ancestors_of(registry, target, memo, visiting) {
            push_unique(&mut result, ancestor);
        }
    }

    visiting.remove(&id);
    memo.insert(id, result.clone());
    result
}

fn push_unique(values: &mut Vec<ObjectId>, value: ObjectId) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn inherit_docstrings(registry: &mut Registry) {
    let methods: Vec<ObjectId> = registry
        .objects()
        .iter()
        .filter(|object| object.kind == ObjectKind::Method)
        .map(|object| object.id)
        .collect();

    for id in methods {
        let (namespace, name, scope) = {
            let object = registry.get(id);
            let KindData::Method { scope, .. } = &object.extra else {
                continue;
            };
            (object.namespace, object.name.clone(), *scope)
        };
        let Some(namespace) = namespace else {
            continue;
        };

        let ancestors = match &registry.get(namespace).extra {
            KindData::Namespace { ancestors, .. } => ancestors.clone(),
            _ => Vec::new(),
        };

        let mut inherited = Docstring::default();
        let mut seen = HashSet::new();
        for ancestor in ancestors {
            let prefix = registry.get(ancestor).path.clone();
            let path = join_method_path(&prefix, scope.separator(), &name);
            let Some(target) = registry.id_for_path_bytes(path.as_bytes()) else {
                continue;
            };
            if target == id || !seen.insert(target) {
                continue;
            }
            let candidate = registry.get(target).docstring.clone();
            merge_docstring(&mut inherited, &candidate);
        }

        let mut docstring = registry.get(id).docstring.clone();
        merge_docstring(&mut docstring, &inherited);
        if docstring != registry.get(id).docstring {
            registry.get_mut(id).docstring = docstring;
        }
    }
}

fn merge_docstring(into: &mut Docstring, from: &Docstring) {
    if into.text.is_empty() {
        into.text = from.text.clone();
        into.summary = from.summary.clone();
    }
    for tag in &from.tags {
        let duplicate = into
            .tags
            .iter()
            .any(|existing| existing.tag == tag.tag && existing.name == tag.name);
        if !duplicate {
            into.tags.push(tag.clone());
        }
    }
}

fn apply_see_references(registry: &mut Registry, warnings: &mut Vec<ResolutionWarning>) {
    let ids: Vec<ObjectId> = registry.objects().iter().map(|object| object.id).collect();

    for id in ids {
        let object = registry.get(id);
        let Some(target_name) = see_target(&object.docstring.text) else {
            continue;
        };
        let namespace = object.namespace;
        let Some(target) = resolve_name(registry, &target_name, namespace) else {
            warnings.push(ResolutionWarning {
                message: format!(
                    "(see {}) could not be resolved",
                    target_name.to_string_lossy()
                ),
                object: Some(id),
            });
            continue;
        };
        if target == id {
            continue;
        }
        let target_docstring = registry.get(target).docstring.clone();
        let object = registry.get_mut(id);
        let mut docstring = target_docstring;
        merge_docstring(&mut docstring, &object.docstring);
        object.docstring = docstring;
    }
}

fn see_target(text: &SourceText) -> Option<SourceText> {
    let bytes = text.as_bytes().trim_ascii_start();
    let body = bytes.strip_prefix(b"(see ")?;
    let end = body.iter().position(|byte| *byte == b')')?;
    Some(SourceText::from_bytes(&body[..end]))
}

fn join_method_path(prefix: &SourceText, separator: &str, name: &SourceText) -> SourceText {
    let mut result = Vec::with_capacity(prefix.len() + separator.len() + name.len());
    if !prefix.is_empty() {
        result.extend_from_slice(prefix.as_bytes());
        result.extend_from_slice(separator.as_bytes());
    }
    result.extend_from_slice(name.as_bytes());
    SourceText::from_vec(result)
}
