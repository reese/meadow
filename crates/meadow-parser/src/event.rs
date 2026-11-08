//! Owned, serializable events produced by the parser stage.
//!
//! The parser never exposes Prism nodes outside this crate. Every event
//! contains owned, byte-preserving source text and plain numeric source ranges
//! so that it can be sent across threads, cached, and serialized for comparison
//! in tests. Text is stored as [`SourceText`], never as an assumed UTF-8
//! `String`; callers convert lossily only when they render it.

use serde::{Deserialize, Serialize};

use crate::{SourceLocation, SourceText};

/// A comment block attached to a code object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Docstring {
    /// Comment text with comment markers removed and relative indentation kept.
    pub text: SourceText,
    /// Location of the comment block in the source.
    pub location: SourceLocation,
}

/// A class or module body opening event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassOpenEvent {
    /// Fully qualified constant path as written (`Foo::Bar`).
    pub name: SourceText,
    /// Superclass expression as written, when present.
    pub superclass: Option<SourceText>,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the whole `class` expression.
    pub location: SourceLocation,
}

/// A class body closing event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCloseEvent {
    /// Name of the class that is closing.
    pub name: SourceText,
    /// Location of the `end` keyword.
    pub location: SourceLocation,
}

/// A module body opening event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleOpenEvent {
    /// Fully qualified constant path as written (`Foo::Bar`).
    pub name: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the whole `module` expression.
    pub location: SourceLocation,
}

/// A module body closing event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleCloseEvent {
    /// Name of the module that is closing.
    pub name: SourceText,
    /// Location of the `end` keyword.
    pub location: SourceLocation,
}

/// A singleton class (`class << self`) opening event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingletonClassOpenEvent {
    /// Receiver expression, as written.
    pub receiver: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the whole `class << receiver` expression.
    pub location: SourceLocation,
}

/// A singleton class closing event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingletonClassCloseEvent {
    /// Receiver expression, as written.
    pub receiver: SourceText,
    /// Location of the `end` keyword.
    pub location: SourceLocation,
}

/// Whether a method is defined at instance or class/singleton scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodScope {
    /// An instance method.
    Instance,
    /// A class, module, or singleton method.
    Class,
}

/// The kind of a method parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// `name`
    Required,
    /// `name = default`
    Optional,
    /// `*name` or `*`
    Rest,
    /// A required parameter appearing after the rest parameter.
    Post,
    /// `name:`
    Keyword,
    /// `name: default`
    KeywordOptional,
    /// `**name`, `**`, or `**nil`
    KeywordRest,
    /// `&name` or `&`
    Block,
    /// `...`
    Forwarding,
}

/// A method parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    /// Parameter name, without sigils. Empty for anonymous parameters.
    pub name: SourceText,
    /// Parameter kind.
    pub kind: ParamKind,
    /// Source text of the default value, when present.
    pub default: Option<SourceText>,
}

/// A method definition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodDefEvent {
    /// Method name.
    pub name: SourceText,
    /// Instance or class/singleton scope.
    pub scope: MethodScope,
    /// Explicit receiver text (`self` or a constant path), when present.
    pub receiver: Option<SourceText>,
    /// Parsed parameters in source order.
    pub parameters: Vec<Param>,
    /// Rendered signature, for example `foo(a, b = 1, *rest, key:)`.
    pub signature: SourceText,
    /// Complete source text of the method definition.
    pub source: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the whole `def` expression.
    pub location: SourceLocation,
}

/// A constant assignment event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstantWriteEvent {
    /// Constant path as written.
    pub path: SourceText,
    /// Source text of the assigned value.
    pub value_source: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the assignment expression.
    pub location: SourceLocation,
}

/// An instance variable assignment event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceVariableWriteEvent {
    /// Instance variable name including its `@` sigil.
    pub name: SourceText,
    /// Source text of the assigned value.
    pub value_source: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the assignment expression.
    pub location: SourceLocation,
}

/// A class variable assignment event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassVariableWriteEvent {
    /// Class variable name including its `@@` sigil.
    pub name: SourceText,
    /// Source text of the assigned value, when present.
    pub value_source: Option<SourceText>,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the assignment expression.
    pub location: SourceLocation,
}

/// A single call argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Argument {
    /// Complete source text of the argument.
    pub source: SourceText,
    /// Unescaped literal value for symbol and string arguments.
    pub literal: Option<SourceText>,
}

/// A block passed to a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockEvent {
    /// Block parameters.
    pub parameters: Vec<Param>,
    /// Complete source text of the block.
    pub source: SourceText,
}

/// A method call event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEvent {
    /// Complete source text of the call.
    pub source: SourceText,
    /// Explicit receiver text, when present.
    pub receiver: Option<SourceText>,
    /// Called method name.
    pub name: SourceText,
    /// Arguments in source order.
    pub arguments: Vec<Argument>,
    /// Attached block, when present.
    pub block: Option<BlockEvent>,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the whole call expression.
    pub location: SourceLocation,
}

/// An `alias` or `alias_method` style event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasEvent {
    /// New method name.
    pub new_name: SourceText,
    /// Existing method name being aliased.
    pub old_name: SourceText,
    /// Attached comment block, if any.
    pub docstring: Option<Docstring>,
    /// Location of the alias expression.
    pub location: SourceLocation,
}

/// A free-standing comment that was not attached to a code object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentEvent {
    /// Comment text with comment markers removed and relative indentation kept.
    pub text: SourceText,
    /// Location of the comment block.
    pub location: SourceLocation,
}

/// An event emitted by the parser in document order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    /// A class body opens.
    ClassOpen(ClassOpenEvent),
    /// A class body closes.
    ClassClose(ClassCloseEvent),
    /// A module body opens.
    ModuleOpen(ModuleOpenEvent),
    /// A module body closes.
    ModuleClose(ModuleCloseEvent),
    /// A singleton class body opens.
    SingletonClassOpen(SingletonClassOpenEvent),
    /// A singleton class body closes.
    SingletonClassClose(SingletonClassCloseEvent),
    /// A method is defined.
    MethodDef(MethodDefEvent),
    /// A constant is assigned.
    ConstantWrite(ConstantWriteEvent),
    /// A class variable is assigned.
    ClassVariableWrite(ClassVariableWriteEvent),
    /// An instance variable is assigned.
    InstanceVariableWrite(InstanceVariableWriteEvent),
    /// A method call is made.
    Call(CallEvent),
    /// A method alias is created.
    Alias(AliasEvent),
    /// A free-standing comment.
    Comment(CommentEvent),
}

impl Event {
    /// Returns the source location associated with this event.
    #[must_use]
    pub const fn location(&self) -> &SourceLocation {
        match self {
            Self::ClassOpen(e) => &e.location,
            Self::ClassClose(e) => &e.location,
            Self::ModuleOpen(e) => &e.location,
            Self::ModuleClose(e) => &e.location,
            Self::SingletonClassOpen(e) => &e.location,
            Self::SingletonClassClose(e) => &e.location,
            Self::MethodDef(e) => &e.location,
            Self::ConstantWrite(e) => &e.location,
            Self::ClassVariableWrite(e) => &e.location,
            Self::InstanceVariableWrite(e) => &e.location,
            Self::Call(e) => &e.location,
            Self::Alias(e) => &e.location,
            Self::Comment(e) => &e.location,
        }
    }
}

/// Severity of a parser diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    /// A syntax error. Prism may still have recovered a partial tree.
    Error,
    /// A parser warning.
    Warning,
}

/// A diagnostic emitted while parsing a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Diagnostic severity.
    pub level: DiagnosticLevel,
    /// Human-readable message.
    pub message: SourceText,
    /// Location in the source, when available.
    pub location: Option<SourceLocation>,
}
