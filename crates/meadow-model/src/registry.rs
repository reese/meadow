//! The arena-backed code object registry.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::object::{
    CodeObject, KindData, MixinEntry, ObjectId, ObjectKind, Ref, Scope, Visibility,
};
use crate::source::{SourceLocation, SourceText};

/// A deterministic, serializable graph of documented code objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    format_version: u32,
    meadow_version: SourceText,
    objects: Vec<CodeObject>,
    #[serde(with = "path_map")]
    by_path: BTreeMap<SourceText, ObjectId>,
    #[serde(with = "path_map")]
    files: BTreeMap<SourceText, Vec<ObjectId>>,
    requires: Vec<RequireRecord>,
}

/// The kind of a recorded load statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequireKind {
    /// `require`
    Require,
    /// `require_relative`
    RequireRelative,
    /// `autoload`
    Autoload,
}

/// A require/autoload call recorded without following it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequireRecord {
    /// Kind of load call.
    pub kind: RequireKind,
    /// Arguments as written in the source.
    pub arguments: Vec<SourceText>,
    /// File containing the call.
    pub file: SourceText,
    /// Call source location.
    pub location: SourceLocation,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// Creates a registry containing only the root object.
    #[must_use]
    pub fn new() -> Self {
        let root = CodeObject::new(ObjectKind::Root, "", "", None, KindData::empty_namespace());
        let mut registry = Self {
            format_version: 1,
            meadow_version: SourceText::from(env!("CARGO_PKG_VERSION")),
            objects: Vec::new(),
            by_path: BTreeMap::new(),
            files: BTreeMap::new(),
            requires: Vec::new(),
        };
        registry.push(root);
        registry
    }

    /// Returns the root object.
    #[must_use]
    pub fn root(&self) -> &CodeObject {
        self.get(ObjectId::ROOT)
    }

    /// Returns all objects in insertion order.
    #[must_use]
    pub fn objects(&self) -> &[CodeObject] {
        &self.objects
    }

    /// Returns all objects in insertion order.
    #[must_use]
    pub fn objects_mut(&mut self) -> &mut [CodeObject] {
        &mut self.objects
    }

    /// Returns the object for an identifier.
    ///
    /// # Panics
    ///
    /// Panics if the identifier does not belong to this registry.
    #[must_use]
    pub fn get(&self, id: ObjectId) -> &CodeObject {
        &self.objects[id.index()]
    }

    /// Returns the mutable object for an identifier.
    ///
    /// # Panics
    ///
    /// Panics if the identifier does not belong to this registry.
    #[must_use]
    pub fn get_mut(&mut self, id: ObjectId) -> &mut CodeObject {
        &mut self.objects[id.index()]
    }

    /// Looks up an object by its YARD-style path.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<&CodeObject> {
        self.id_for_path(path).map(|id| self.get(id))
    }

    /// Looks up an object identifier by path.
    #[must_use]
    pub fn id_for_path(&self, path: &str) -> Option<ObjectId> {
        self.id_for_path_bytes(path.as_bytes())
    }

    /// Looks up an object identifier by raw path bytes.
    #[must_use]
    pub fn id_for_path_bytes(&self, path: &[u8]) -> Option<ObjectId> {
        self.by_path.get(path).copied()
    }

    /// Returns the registry's path index.
    #[must_use]
    pub const fn path_index(&self) -> &BTreeMap<SourceText, ObjectId> {
        &self.by_path
    }

    /// Returns objects grouped by the file that contributed them.
    #[must_use]
    pub const fn files(&self) -> &BTreeMap<SourceText, Vec<ObjectId>> {
        &self.files
    }

    /// Returns the registry format version.
    #[must_use]
    pub const fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Returns the meadow version that wrote the registry.
    #[must_use]
    pub fn meadow_version(&self) -> &SourceText {
        &self.meadow_version
    }

    /// Returns recorded require/autoload calls.
    #[must_use]
    pub fn requires(&self) -> &[RequireRecord] {
        &self.requires
    }

    /// Records a require/autoload call.
    pub fn add_require(&mut self, record: RequireRecord) {
        if !self.requires.contains(&record) {
            self.requires.push(record);
        }
    }

    /// Adds an object and assigns its identifier.
    ///
    /// # Panics
    ///
    /// Panics if an object with the same path is already registered. Callers
    /// that reopen an object should update the existing object instead.
    pub fn push(&mut self, mut object: CodeObject) -> ObjectId {
        let id =
            ObjectId(u32::try_from(self.objects.len()).expect("registry exceeds u32::MAX objects"));
        assert!(
            !self.by_path.contains_key(object.path.as_bytes()),
            "duplicate registry path: {}",
            object.path
        );

        object.id = id;
        self.by_path.insert(object.path.clone(), id);
        self.objects.push(object);
        id
    }

    /// Records that `file` contributed `id`.
    pub fn register_file(&mut self, file: &SourceText, id: ObjectId) {
        let ids = self.files.entry(file.clone()).or_default();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    /// Adds a source range to an object, ignoring exact duplicates.
    pub fn add_source(&mut self, id: ObjectId, location: SourceLocation) {
        let object = self.get_mut(id);
        if !object.sources.contains(&location) {
            object.sources.push(location);
        }
    }

    /// Adds a child to a namespace object, avoiding duplicates.
    pub fn add_child(&mut self, namespace: ObjectId, child: ObjectId) {
        if let KindData::Namespace { children, .. } = &mut self.get_mut(namespace).extra
            && !children.contains(&child)
        {
            children.push(child);
        }
    }

    /// Returns the direct children of a namespace.
    #[must_use]
    pub fn children(&self, namespace: ObjectId) -> &[ObjectId] {
        match &self.get(namespace).extra {
            KindData::Namespace { children, .. } => children,
            _ => &[],
        }
    }

    /// Appends a mixin entry to a namespace.
    pub fn add_mixin(&mut self, namespace: ObjectId, kind: crate::object::MixinKind, target: Ref) {
        let entry = MixinEntry {
            kind,
            target: target.clone(),
        };
        if let KindData::Namespace {
            includes,
            extends,
            prepends,
            mixin_order,
            ..
        } = &mut self.get_mut(namespace).extra
        {
            match kind {
                crate::object::MixinKind::Include => includes.push(target),
                crate::object::MixinKind::Extend => extends.push(target),
                crate::object::MixinKind::Prepend => prepends.push(target),
            }
            mixin_order.push(entry);
        }
    }

    /// Removes objects contributed exclusively by the given files and remaps
    /// all surviving identifiers.
    ///
    /// Objects also owned by another file (for example a reopened class) are
    /// kept. References to removed objects become proxies.
    ///
    /// # Panics
    ///
    /// Panics if the surviving object count exceeds `u32::MAX`.
    pub fn remove_files(&mut self, files: &BTreeSet<SourceText>) {
        let old_paths: BTreeMap<ObjectId, SourceText> = self
            .objects
            .iter()
            .map(|object| (object.id, object.path.clone()))
            .collect();
        let mut remove = BTreeSet::new();
        for (id, owners) in &ownership_map(&self.files) {
            if !owners.is_empty() && owners.iter().all(|file| files.contains(file)) {
                remove.insert(*id);
            }
        }

        let mut remap = BTreeMap::new();
        for object in &self.objects {
            if !remove.contains(&object.id) {
                remap.insert(
                    object.id,
                    ObjectId(u32::try_from(remap.len()).expect("id overflow")),
                );
            }
        }

        let mut objects = Vec::with_capacity(remap.len());
        for mut object in self.objects.drain(..) {
            let Some(new_id) = remap.get(&object.id).copied() else {
                continue;
            };
            let old_id = object.id;
            object.id = new_id;
            object.namespace = object.namespace.and_then(|id| remap.get(&id).copied());
            remap_kind(&mut object.extra, &remap, &old_paths, &remove);
            let _ = new_id;
            let _ = old_id;
            objects.push(object);
        }
        self.objects = objects;

        let mut new_files = BTreeMap::new();
        for (file, ids) in self.files.clone() {
            if files.contains(&file) {
                continue;
            }
            let remapped: Vec<ObjectId> =
                ids.iter().filter_map(|id| remap.get(id).copied()).collect();
            new_files.insert(file, remapped);
        }
        self.files = new_files;
        self.requires.retain(|record| !files.contains(&record.file));
        self.rebuild_index();
    }

    /// Rebuilds the path index. Useful after deserializing a registry from a
    /// format that omits the index.
    pub fn rebuild_index(&mut self) {
        self.by_path.clear();
        for object in &self.objects {
            self.by_path.insert(object.path.clone(), object.id);
        }
    }

    /// Returns the object's scope when it is a method.
    #[must_use]
    pub fn method_scope(&self, id: ObjectId) -> Option<Scope> {
        match &self.get(id).extra {
            KindData::Method { scope, .. } => Some(*scope),
            _ => None,
        }
    }

    /// Returns the object's visibility, defaulting to public for non-objects.
    #[must_use]
    pub fn visibility(&self, id: ObjectId) -> Visibility {
        self.get(id).visibility
    }
}

fn ownership_map(
    files: &BTreeMap<SourceText, Vec<ObjectId>>,
) -> BTreeMap<ObjectId, Vec<SourceText>> {
    let mut owners: BTreeMap<ObjectId, Vec<SourceText>> = BTreeMap::new();
    for (file, ids) in files {
        for id in ids {
            owners.entry(*id).or_default().push(file.clone());
        }
    }
    owners
}

fn remap_ref(
    reference: &mut Ref,
    remap: &BTreeMap<ObjectId, ObjectId>,
    old_paths: &BTreeMap<ObjectId, SourceText>,
    remove: &BTreeSet<ObjectId>,
) {
    if let Ref::Resolved(id) = reference {
        if let Some(new_id) = remap.get(id) {
            *id = *new_id;
        } else if remove.contains(id) {
            *reference = Ref::Proxy {
                name: old_paths.get(id).cloned().unwrap_or_default(),
                namespace_hint: None,
            };
        }
    }
}

fn remap_kind(
    extra: &mut KindData,
    remap: &BTreeMap<ObjectId, ObjectId>,
    old_paths: &BTreeMap<ObjectId, SourceText>,
    remove: &BTreeSet<ObjectId>,
) {
    match extra {
        KindData::Namespace {
            superclass,
            includes,
            extends,
            prepends,
            mixin_order,
            ancestors,
            children,
            ..
        } => {
            if let Some(reference) = superclass {
                remap_ref(reference, remap, old_paths, remove);
            }
            for reference in includes
                .iter_mut()
                .chain(extends.iter_mut())
                .chain(prepends.iter_mut())
            {
                remap_ref(reference, remap, old_paths, remove);
            }
            for entry in mixin_order {
                remap_ref(&mut entry.target, remap, old_paths, remove);
            }
            ancestors.retain(|id| !remove.contains(id));
            for id in ancestors.iter_mut() {
                if let Some(new_id) = remap.get(id) {
                    *id = *new_id;
                }
            }
            children.retain(|id| !remove.contains(id));
            for id in children.iter_mut() {
                if let Some(new_id) = remap.get(id) {
                    *id = *new_id;
                }
            }
        }
        KindData::Method {
            aliases, alias_of, ..
        } => {
            aliases.retain(|id| !remove.contains(id));
            for id in aliases.iter_mut() {
                if let Some(new_id) = remap.get(id) {
                    *id = *new_id;
                }
            }
            if let Some(id) = alias_of {
                if let Some(new_id) = remap.get(id) {
                    *id = *new_id;
                } else if remove.contains(id) {
                    *alias_of = None;
                }
            }
        }
        KindData::Constant { alias_of, .. } => {
            if let Some(reference) = alias_of {
                remap_ref(reference, remap, old_paths, remove);
            }
        }
        KindData::ClassVar { .. } | KindData::InstanceVariable { .. } | KindData::Macro { .. } => {}
    }
}

mod path_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::source::SourceText;

    pub fn serialize<S, V>(map: &BTreeMap<SourceText, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        serializer.collect_seq(map.iter())
    }

    pub fn deserialize<'de, D, V>(deserializer: D) -> Result<BTreeMap<SourceText, V>, D::Error>
    where
        D: Deserializer<'de>,
        V: Deserialize<'de>,
    {
        let entries = Vec::<(SourceText, V)>::deserialize(deserializer)?;
        Ok(entries.into_iter().collect())
    }
}
