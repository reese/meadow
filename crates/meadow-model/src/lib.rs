//! Code object model, registry, and serialization for meadow.
//!
//! This crate owns the arena-backed [`CodeObject`](https://docs.rs/meadow-model)
//! graph described in the product spec. It intentionally contains no parser
//! types; Parsed source reaches this crate only through owned, serializable
//! structures.

mod object;
mod registry;
mod source;

pub use object::{
    AttrInfo, CodeObject, Docstring, HashStyle, KindData, MixinEntry, MixinKind, ObjectId,
    ObjectKind, Param, ParamKind, Ref, Scope, Tag, TypeExpr, Visibility,
};
pub use registry::{Registry, RequireKind, RequireRecord};
pub use source::{SourceLocation, SourceText};
