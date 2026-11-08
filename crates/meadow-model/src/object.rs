//! Code objects and the data attached to each kind.

use serde::{Deserialize, Serialize};

use crate::source::{SourceLocation, SourceText};

/// Opaque index of a code object inside a registry arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u32);

impl ObjectId {
    /// Identifier of the registry's root object.
    pub const ROOT: Self = Self(0);

    /// Returns the index into the registry's object arena.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The kind of a documented object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// The unnamed registry root.
    Root,
    /// A module namespace.
    Module,
    /// A class namespace.
    Class,
    /// An instance or class method.
    Method,
    /// A constant.
    Constant,
    /// A class variable.
    ClassVariable,
    /// An instance variable with a statically known type.
    InstanceVariable,
    /// A documentation macro.
    Macro,
}

/// Whether a method is an instance method or a class/singleton method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// An instance method.
    Instance,
    /// A class, module, or singleton method.
    Class,
}

impl Scope {
    /// Returns the path separator used before a method name.
    #[must_use]
    pub const fn separator(self) -> &'static str {
        match self {
            Self::Instance => "#",
            Self::Class => ".",
        }
    }
}

/// The visibility of an object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Publicly callable.
    #[default]
    Public,
    /// Callable from the same class hierarchy.
    Protected,
    /// Callable only from within the defining object.
    Private,
}

/// The kind of a method parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// `name`
    Required,
    /// `name = default`
    Optional,
    /// `*name`
    Rest,
    /// A required parameter appearing after the rest parameter.
    Post,
    /// `name:`
    Keyword,
    /// `name: default`
    KeywordOptional,
    /// `**name`
    KeywordRest,
    /// `&name`
    Block,
    /// `...`
    Forwarding,
}

/// A method parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name, without sigils.
    pub name: SourceText,
    /// Parameter kind.
    pub kind: ParamKind,
    /// Source text of a default value, when present.
    pub default: Option<SourceText>,
}

/// A parsed docstring: free text plus an ordered list of tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Docstring {
    /// Free-text body with tag lines removed.
    pub text: SourceText,
    /// First-line summary.
    pub summary: SourceText,
    /// Tags in source order.
    pub tags: Vec<Tag>,
}

/// A single docstring tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tag {
    /// Tag name without the leading `@`.
    pub tag: SourceText,
    /// Parameter, attribute, or hash name, when the tag form has one.
    pub name: Option<SourceText>,
    /// Parsed type expression, when the tag has a valid type list.
    pub types: Option<TypeExpr>,
    /// Raw type text, kept even when parsing failed.
    pub types_source: Option<SourceText>,
    /// Description text, including continuation lines.
    pub text: SourceText,
    /// Nested docstring for `@overload`, when present.
    pub docstring: Option<Box<Docstring>>,
    /// Option key for `@option`.
    pub key: Option<SourceText>,
    /// Option default value for `@option`.
    pub default: Option<SourceText>,
}

/// The syntax used for a hash type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashStyle {
    /// `Hash<Key, Value>`
    Angles,
    /// `Hash{Key=>Value}`
    Braces,
}

/// A structured type expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeExpr {
    /// A class or module name, including namespaced names.
    Name(SourceText),
    /// A literal keyword such as `true`, `false`, `nil`, `void`, `self`, or a symbol.
    Literal(SourceText),
    /// A duck type such as `#each`.
    Duck(SourceText),
    /// A generic type such as `Array<String>`.
    Generic {
        /// Generic base type.
        base: Box<TypeExpr>,
        /// Type parameters.
        params: Vec<TypeExpr>,
    },
    /// A hash type, in either `Hash<K, V>` or `Hash{K=>V}` form.
    Hash {
        /// Key type.
        key: Box<TypeExpr>,
        /// Value type.
        value: Box<TypeExpr>,
        /// Original hash syntax.
        style: HashStyle,
    },
    /// A tuple type such as `(String, Integer)`.
    Tuple(Vec<TypeExpr>),
    /// A comma-separated union of types.
    Union(Vec<TypeExpr>),
}

impl std::fmt::Display for TypeExpr {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(name) | Self::Literal(name) => write!(formatter, "{name}"),
            Self::Duck(name) => write!(formatter, "#{name}"),
            Self::Generic { base, params } => {
                write!(formatter, "{base}<")?;
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{param}")?;
                }
                formatter.write_str(">")
            }
            Self::Hash { key, value, style } => match style {
                HashStyle::Angles => write!(formatter, "Hash<{key}, {value}>"),
                HashStyle::Braces => write!(formatter, "Hash{{{key}=>{value}}}"),
            },
            Self::Tuple(types) => {
                formatter.write_str("(")?;
                for (index, ty) in types.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{ty}")?;
                }
                formatter.write_str(")")
            }
            Self::Union(types) => {
                for (index, ty) in types.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{ty}")?;
                }
                Ok(())
            }
        }
    }
}

/// A reference to another code object, which may be unresolved.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ref {
    /// The reference was resolved to a registry object.
    Resolved(ObjectId),
    /// The reference points outside the registry.
    Proxy {
        /// Name as written in the source.
        name: SourceText,
        /// Namespace in which resolution was attempted.
        namespace_hint: Option<ObjectId>,
    },
}

impl Ref {
    /// Returns the resolved object identifier, if any.
    #[must_use]
    pub const fn resolved_id(&self) -> Option<ObjectId> {
        match self {
            Self::Resolved(id) => Some(*id),
            Self::Proxy { .. } => None,
        }
    }
}

/// The kind of a mixin entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixinKind {
    /// `include`
    Include,
    /// `extend`
    Extend,
    /// `prepend`
    Prepend,
}

/// One entry in a namespace's ordered mixin list.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MixinEntry {
    /// Whether the entry came from `include`, `extend`, or `prepend`.
    pub kind: MixinKind,
    /// The mixed-in namespace.
    pub target: Ref,
}

/// Information about a method generated from an attribute declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttrInfo {
    /// Attribute name.
    pub name: SourceText,
    /// Whether the attribute has a reader.
    pub reader: bool,
    /// Whether the attribute has a writer.
    pub writer: bool,
}

/// Kind-specific data attached to a code object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KindData {
    /// Namespace data for a module or class.
    Namespace {
        /// Declared superclass, if any.
        superclass: Option<Ref>,
        /// Modules brought in with `include`, in declaration order.
        includes: Vec<Ref>,
        /// Modules brought in with `extend`, in declaration order.
        extends: Vec<Ref>,
        /// Modules brought in with `prepend`, in declaration order.
        prepends: Vec<Ref>,
        /// Includes, extends, and prepends in a single ordered list.
        mixin_order: Vec<MixinEntry>,
        /// Resolved ancestors in method-lookup order.
        ancestors: Vec<ObjectId>,
        /// Direct child objects.
        children: Vec<ObjectId>,
        /// Constants made private with `private_constant`.
        constants_private: Vec<SourceText>,
    },
    /// Data for an instance or class method.
    Method {
        /// Instance or class scope.
        scope: Scope,
        /// Parameters in source order.
        params: Vec<Param>,
        /// Full method source.
        source: SourceText,
        /// Rendered signature.
        signature: SourceText,
        /// Whether the method came from an explicit `def`.
        explicit: bool,
        /// Methods aliased to this one.
        aliases: Vec<ObjectId>,
        /// The method this alias points at, when resolvable.
        alias_of: Option<ObjectId>,
        /// Attribute metadata, when generated from `attr_*`.
        attr: Option<AttrInfo>,
    },
    /// Data for a constant.
    Constant {
        /// Source text of the assigned value.
        value_source: SourceText,
        /// Constant this one aliases, when the value is another constant.
        alias_of: Option<Ref>,
    },
    /// Data for a class variable.
    ClassVar {
        /// Source text of the assigned value, when present.
        value_source: Option<SourceText>,
    },
    /// Data for an instance variable.
    InstanceVariable {
        /// Source text of the assigned value, when present.
        value_source: Option<SourceText>,
    },
    /// Data for a documentation macro.
    Macro {
        /// Macro name.
        name: SourceText,
        /// Macro body.
        body: SourceText,
        /// Whether the macro is attached to the following call.
        attached: bool,
    },
}

impl KindData {
    /// Returns true for namespace kinds.
    #[must_use]
    pub const fn is_namespace(&self) -> bool {
        matches!(self, Self::Namespace { .. })
    }

    /// Creates empty namespace data.
    #[must_use]
    pub const fn empty_namespace() -> Self {
        Self::Namespace {
            superclass: None,
            includes: Vec::new(),
            extends: Vec::new(),
            prepends: Vec::new(),
            mixin_order: Vec::new(),
            ancestors: Vec::new(),
            children: Vec::new(),
            constants_private: Vec::new(),
        }
    }
}

/// A documented code object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeObject {
    /// Arena identifier.
    pub id: ObjectId,
    /// Object kind.
    pub kind: ObjectKind,
    /// Unqualified object name.
    pub name: SourceText,
    /// Fully qualified YARD-style path.
    pub path: SourceText,
    /// Enclosing namespace, or `None` for the root.
    pub namespace: Option<ObjectId>,
    /// Raw docstring.
    pub docstring: Docstring,
    /// Every source range contributing to this object.
    pub sources: Vec<SourceLocation>,
    /// Object visibility.
    pub visibility: Visibility,
    /// Whether the object was created by metaprogramming or a directive.
    pub dynamic: bool,
    /// Active `@!group`, when any.
    pub group: Option<SourceText>,
    /// Kind-specific data.
    pub extra: KindData,
}

impl CodeObject {
    /// Builds an object with a placeholder id. [`crate::Registry::push`] assigns
    /// the real identifier.
    #[must_use]
    pub fn new(
        kind: ObjectKind,
        name: impl Into<SourceText>,
        path: impl Into<SourceText>,
        namespace: Option<ObjectId>,
        extra: KindData,
    ) -> Self {
        Self {
            id: ObjectId::ROOT,
            kind,
            name: name.into(),
            path: path.into(),
            namespace,
            docstring: Docstring::default(),
            sources: Vec::new(),
            visibility: Visibility::Public,
            dynamic: false,
            group: None,
            extra,
        }
    }
}
