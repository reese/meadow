//! `ActiveRecord` model, association, scope, enum, and schema support.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use meadow_model::{
    AttrInfo, Docstring, HashStyle, KindData, ObjectId, Param, ParamKind, Registry, Scope,
    SourceText, TypeExpr,
};
use meadow_parser::Event;

use crate::util::{MethodDetails, add_method, superclass_name, typed_tag};
use crate::{Handled, Plugin, PluginContext};

const ASSOCIATIONS: &[&[u8]] = &[
    b"belongs_to",
    b"has_one",
    b"has_many",
    b"has_and_belongs_to_many",
];

#[derive(Default)]
struct ArState {
    calls: Vec<ArCall>,
    warnings: Vec<String>,
}

struct ArCall {
    namespace: ObjectId,
    name: SourceText,
    args: Vec<SourceText>,
    literals: Vec<Option<SourceText>>,
    source: SourceText,
}

struct Column {
    name: String,
    types: TypeExpr,
}

type CallView = (
    ObjectId,
    SourceText,
    Vec<SourceText>,
    Vec<Option<SourceText>>,
    SourceText,
);

/// `ActiveRecord` support for models, associations, scopes, enums, and schema columns.
pub struct ActiveRecord {
    state: Mutex<ArState>,
    project_root: PathBuf,
    schema_path: PathBuf,
    model_superclasses: Vec<String>,
    irregulars: BTreeMap<String, String>,
}

impl ActiveRecord {
    /// Creates the plugin rooted at a project directory.
    #[must_use]
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        Self::with_options(&project_root, None, Vec::new(), BTreeMap::new())
    }

    /// Creates the plugin with configurable schema, model superclasses, and irregulars.
    #[must_use]
    pub fn with_options(
        project_root: &Path,
        schema: Option<PathBuf>,
        model_superclasses: Vec<String>,
        irregulars: BTreeMap<String, String>,
    ) -> Self {
        Self {
            state: Mutex::new(ArState::default()),
            project_root: project_root.to_path_buf(),
            schema_path: schema.unwrap_or_else(|| project_root.join("db/schema.rb")),
            model_superclasses: if model_superclasses.is_empty() {
                vec![
                    "ActiveRecord::Base".to_string(),
                    "ApplicationRecord".to_string(),
                ]
            } else {
                model_superclasses
            },
            irregulars,
        }
    }
}

impl Plugin for ActiveRecord {
    fn name(&self) -> &'static str {
        "activerecord"
    }

    fn warnings(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("activerecord state lock")
            .warnings
            .clone()
    }

    fn on_event(&self, event: &Event, context: &PluginContext<'_>) -> Handled {
        if let Event::Call(call) = event {
            let name = call.name.as_bytes();
            let interesting = ASSOCIATIONS.contains(&name)
                || matches!(
                    name,
                    b"scope"
                        | b"enum"
                        | b"table_name="
                        | b"validates"
                        | b"before_save"
                        | b"after_save"
                );
            let receiver_ok = call.receiver.is_none()
                || (name == b"table_name="
                    && call
                        .receiver
                        .as_ref()
                        .is_some_and(|receiver| receiver.as_bytes() == b"self"));
            if interesting && receiver_ok {
                let mut state = self.state.lock().expect("activerecord state lock");
                state.calls.push(ArCall {
                    namespace: context.namespace,
                    name: call.name.clone(),
                    args: call
                        .arguments
                        .iter()
                        .map(|argument| argument.source.clone())
                        .collect(),
                    literals: call
                        .arguments
                        .iter()
                        .map(|argument| argument.literal.clone())
                        .collect(),
                    source: call.source.clone(),
                });
            }
        }
        Handled::No
    }

    fn after_handle(&self, registry: &mut Registry) {
        let schema = parse_schema(&self.schema_path);
        if schema.is_empty() {
            let structure = self.project_root.join("db/structure.sql");
            if structure.is_file() {
                let mut state = self.state.lock().expect("activerecord state lock");
                if !state
                    .warnings
                    .iter()
                    .any(|warning| warning.contains("structure.sql"))
                {
                    state.warnings.push(
                        "ActiveRecord schema support requires db/schema.rb; db/structure.sql is not supported".to_string(),
                    );
                }
            }
        }
        let calls: Vec<CallView> = {
            let state = self.state.lock().expect("activerecord state lock");
            state
                .calls
                .iter()
                .map(|call| {
                    (
                        call.namespace,
                        call.name.clone(),
                        call.args.clone(),
                        call.literals.clone(),
                        call.source.clone(),
                    )
                })
                .collect()
        };

        let models: Vec<ObjectId> = registry
            .objects()
            .iter()
            .filter(|object| {
                object.extra.is_namespace()
                    && is_model(registry, object.id, &self.model_superclasses)
            })
            .map(|object| object.id)
            .collect();

        let mut table_overrides = BTreeMap::new();
        for (namespace, name, args, literals, _) in &calls {
            if !models.contains(namespace) {
                continue;
            }
            match name.as_bytes() {
                b"table_name=" => {
                    if let Some(table) = literals.first().and_then(Clone::clone) {
                        table_overrides.insert(*namespace, table.to_string_lossy().into_owned());
                    }
                }
                b"belongs_to" | b"has_one" | b"has_many" | b"has_and_belongs_to_many" => {
                    add_association(
                        registry,
                        *namespace,
                        name.as_bytes(),
                        args,
                        literals,
                        &self.irregulars,
                    );
                }
                b"scope" => add_scope(registry, *namespace, args, literals),
                b"enum" => add_enum(registry, *namespace, args, literals),
                _ => {}
            }
        }

        for model in models {
            let table = table_overrides.get(&model).cloned().unwrap_or_else(|| {
                pluralize(&snake_case(&model_name(registry, model)), &self.irregulars)
            });
            if let Some(columns) = schema.get(&table) {
                for column in columns {
                    add_column(registry, model, column);
                }
            }
        }
    }
}

fn add_association(
    registry: &mut Registry,
    namespace: ObjectId,
    kind: &[u8],
    args: &[SourceText],
    literals: &[Option<SourceText>],
    irregulars: &BTreeMap<String, String>,
) {
    let Some(name) = literals.first().and_then(Clone::clone) else {
        return;
    };
    let type_name = option_value(args, "class_name").unwrap_or_else(|| {
        camelize(
            &singularize(&name.to_string_lossy(), irregulars),
            irregulars,
        )
    });
    let types = TypeExpr::Name(SourceText::from(type_name.as_str()));

    let mut methods = vec![(name.clone(), Scope::Instance, Vec::new())];
    methods.push((
        append(&name, "="),
        Scope::Instance,
        vec![Param {
            name: SourceText::from("value"),
            kind: ParamKind::Required,
            default: None,
        }],
    ));
    if kind == b"belongs_to" {
        methods.push((append(&name, "_id"), Scope::Instance, Vec::new()));
        methods.push((
            format_source("build_", &name),
            Scope::Instance,
            vec![Param {
                name: SourceText::from("attributes"),
                kind: ParamKind::KeywordRest,
                default: None,
            }],
        ));
        methods.push((
            format_source("create_", &name),
            Scope::Instance,
            vec![Param {
                name: SourceText::from("attributes"),
                kind: ParamKind::KeywordRest,
                default: None,
            }],
        ));
        methods.push((format_source("reload_", &name), Scope::Instance, Vec::new()));
    } else if kind == b"has_one" {
        methods.push((
            format_source("build_", &name),
            Scope::Instance,
            vec![Param {
                name: SourceText::from("attributes"),
                kind: ParamKind::KeywordRest,
                default: None,
            }],
        ));
        methods.push((
            format_source("create_", &name),
            Scope::Instance,
            vec![Param {
                name: SourceText::from("attributes"),
                kind: ParamKind::KeywordRest,
                default: None,
            }],
        ));
    } else {
        methods.push((append(&name, "_ids"), Scope::Instance, Vec::new()));
        methods.push((
            append(&name, "_ids="),
            Scope::Instance,
            vec![Param {
                name: SourceText::from("ids"),
                kind: ParamKind::Required,
                default: None,
            }],
        ));
    }

    for (method_name, scope, params) in methods {
        let return_tag = typed_tag("return", types.clone(), "");
        let mut docstring = Docstring::default();
        docstring.tags.push(return_tag);
        add_method(
            registry,
            namespace,
            &method_name,
            scope,
            MethodDetails {
                params,
                docstring,
                ..MethodDetails::default()
            },
        );
    }
}

fn add_scope(
    registry: &mut Registry,
    namespace: ObjectId,
    args: &[SourceText],
    literals: &[Option<SourceText>],
) {
    let Some(name) = literals.first().and_then(Clone::clone) else {
        return;
    };
    let params = args
        .get(1)
        .map_or_else(Vec::new, |lambda| lambda_params(&lambda.to_string_lossy()));
    let mut docstring = Docstring::default();
    docstring.tags.push(typed_tag(
        "return",
        TypeExpr::Name(SourceText::from("ActiveRecord::Relation")),
        "",
    ));
    add_method(
        registry,
        namespace,
        &name,
        Scope::Class,
        MethodDetails {
            params,
            docstring,
            ..MethodDetails::default()
        },
    );
}

fn add_enum(
    registry: &mut Registry,
    namespace: ObjectId,
    args: &[SourceText],
    literals: &[Option<SourceText>],
) {
    let (status, values_source) = if let Some(status) = literals.first().and_then(Clone::clone) {
        (status, args.get(1).cloned().unwrap_or_default())
    } else if let Some(first) = args.first() {
        let text = first.to_string_lossy();
        if let Some(position) = text.find(':') {
            (
                SourceText::from(text[..position].trim()),
                SourceText::from(text[position + 1..].trim()),
            )
        } else {
            (SourceText::from("status"), first.clone())
        }
    } else {
        return;
    };
    let values = enum_values(&values_source.to_string_lossy());
    if values.is_empty() {
        return;
    }

    let prefix = enum_affix(args, "prefix", &status);
    let suffix = enum_affix(args, "suffix", &status);
    add_enum_value_methods(
        registry,
        namespace,
        &values,
        prefix.as_deref(),
        suffix.as_deref(),
    );

    let statuses = SourceText::from("statuses");
    let mut statuses_doc = Docstring::default();
    statuses_doc.tags.push(typed_tag(
        "return",
        TypeExpr::Hash {
            key: Box::new(TypeExpr::Name(SourceText::from("Symbol"))),
            value: Box::new(TypeExpr::Name(SourceText::from("Integer"))),
            style: HashStyle::Braces,
        },
        "",
    ));
    add_method(
        registry,
        namespace,
        &statuses,
        Scope::Class,
        MethodDetails {
            docstring: statuses_doc,
            ..MethodDetails::default()
        },
    );

    let status_reader = status;
    let mut status_doc = Docstring::default();
    status_doc.tags.push(typed_tag(
        "return",
        TypeExpr::Name(SourceText::from("String")),
        "",
    ));
    add_method(
        registry,
        namespace,
        &status_reader,
        Scope::Instance,
        MethodDetails {
            docstring: status_doc,
            ..MethodDetails::default()
        },
    );
}

fn add_enum_value_methods(
    registry: &mut Registry,
    namespace: ObjectId,
    values: &[String],
    prefix: Option<&str>,
    suffix: Option<&str>,
) {
    for value in values {
        let base = match (&prefix, &suffix) {
            (Some(prefix), Some(suffix)) => format!("{prefix}_{value}_{suffix}"),
            (Some(prefix), None) => format!("{prefix}_{value}"),
            (None, Some(suffix)) => format!("{value}_{suffix}"),
            (None, None) => value.clone(),
        };
        let predicate = SourceText::from_vec(format!("{base}?").into_bytes());
        let mut docstring = Docstring::default();
        docstring.tags.push(typed_tag(
            "return",
            TypeExpr::Union(vec![
                TypeExpr::Literal(SourceText::from("true")),
                TypeExpr::Literal(SourceText::from("false")),
            ]),
            "",
        ));
        add_method(
            registry,
            namespace,
            &predicate,
            Scope::Instance,
            MethodDetails {
                docstring: docstring.clone(),
                ..MethodDetails::default()
            },
        );

        let bang = SourceText::from_vec(format!("{base}!").into_bytes());
        add_method(
            registry,
            namespace,
            &bang,
            Scope::Instance,
            MethodDetails {
                docstring,
                ..MethodDetails::default()
            },
        );

        let scope = SourceText::from(base.as_str());
        let mut scope_doc = Docstring::default();
        scope_doc.tags.push(typed_tag(
            "return",
            TypeExpr::Name(SourceText::from("ActiveRecord::Relation")),
            "",
        ));
        add_method(
            registry,
            namespace,
            &scope,
            Scope::Class,
            MethodDetails {
                docstring: scope_doc,
                ..MethodDetails::default()
            },
        );
    }
}

fn add_column(registry: &mut Registry, namespace: ObjectId, column: &Column) {
    let name = SourceText::from(column.name.as_str());
    let mut docstring = Docstring::default();
    docstring
        .tags
        .push(typed_tag("return", column.types.clone(), ""));
    add_method(
        registry,
        namespace,
        &name,
        Scope::Instance,
        MethodDetails {
            docstring: docstring.clone(),
            attr: Some(AttrInfo {
                name: name.clone(),
                reader: true,
                writer: true,
            }),
            ..MethodDetails::default()
        },
    );

    let writer = SourceText::from_vec(format!("{}=", column.name).into_bytes());
    let params = vec![Param {
        name: SourceText::from("value"),
        kind: ParamKind::Required,
        default: None,
    }];
    add_method(
        registry,
        namespace,
        &writer,
        Scope::Instance,
        MethodDetails {
            params,
            docstring,
            attr: Some(AttrInfo {
                name,
                reader: true,
                writer: true,
            }),
            ..MethodDetails::default()
        },
    );
}

fn is_model(registry: &Registry, namespace: ObjectId, names: &[String]) -> bool {
    if superclass_name(registry, namespace).is_some_and(|name| {
        names
            .iter()
            .any(|model| name.as_bytes() == model.as_bytes())
    }) {
        return true;
    }
    match &registry.get(namespace).extra {
        KindData::Namespace { ancestors, .. } => ancestors.iter().any(|id| {
            names
                .iter()
                .any(|model| registry.get(*id).path.as_bytes() == model.as_bytes())
        }),
        _ => false,
    }
}

fn option_value(args: &[SourceText], key: &str) -> Option<String> {
    for argument in args {
        let text = argument.to_string_lossy();
        let needle = format!("{key}:");
        let Some(position) = text.find(&needle) else {
            continue;
        };
        let rest = text[position + needle.len()..].trim();
        let value = rest
            .split([',', '}', '\n'])
            .next()
            .unwrap_or(rest)
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn lambda_params(source: &str) -> Vec<Param> {
    let Some(start) = source.find("->(") else {
        return Vec::new();
    };
    let rest = &source[start + 3..];
    let Some(end) = rest.find(')') else {
        return Vec::new();
    };
    rest[..end]
        .split(',')
        .filter_map(|name| {
            let name = name.trim().trim_start_matches('*').trim_start_matches('&');
            if name.is_empty() {
                None
            } else {
                Some(Param {
                    name: SourceText::from(name),
                    kind: ParamKind::Required,
                    default: None,
                })
            }
        })
        .collect()
}

fn enum_affix(args: &[SourceText], key: &str, status: &SourceText) -> Option<String> {
    let needle = format!("{key}:");
    for argument in args {
        let text = argument.to_string_lossy();
        let Some(position) = text.find(&needle) else {
            continue;
        };
        let rest = text[position + needle.len()..].trim();
        let value = rest
            .split([',', '}', '\n'])
            .next()
            .unwrap_or(rest)
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if value == "true" {
            return Some(status.to_string_lossy().into_owned());
        }
        if !value.is_empty() && value != "false" {
            return Some(value.to_string());
        }
    }
    None
}

fn enum_values(source: &str) -> Vec<String> {
    let Some(start) = source.find('{') else {
        return Vec::new();
    };
    let Some(end) = source.rfind('}') else {
        return Vec::new();
    };
    source[start + 1..end]
        .split(',')
        .filter_map(|entry| {
            let (key, _) = entry.split_once(':')?;
            let key = key.trim().trim_matches('"').trim_matches('\'');
            (!key.is_empty()).then(|| key.to_string())
        })
        .collect()
}

fn parse_schema(path: &Path) -> BTreeMap<String, Vec<Column>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let mut tables: BTreeMap<String, Vec<Column>> = BTreeMap::new();
    let mut current: Option<String> = None;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("create_table") {
            if let Some(name) = table_name(trimmed) {
                tables.entry(name.clone()).or_default();
                current = Some(name);
            }
            continue;
        }
        if current.is_some() && (trimmed == "end" || trimmed.starts_with("end ")) {
            current = None;
            continue;
        }
        let Some(table) = &current else {
            continue;
        };
        let Some(column) = column_from_line(trimmed) else {
            continue;
        };
        tables.entry(table.clone()).or_default().push(column);
    }
    tables
}

fn table_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("create_table")?.trim_start();
    if let Some(rest) = rest.strip_prefix('"') {
        return rest.split('"').next().map(str::to_string);
    }
    if let Some(rest) = rest.strip_prefix(':') {
        return rest
            .split(|character: char| !(character.is_alphanumeric() || character == '_'))
            .next()
            .map(str::to_string);
    }
    None
}

fn column_from_line(line: &str) -> Option<Column> {
    let rest = line.strip_prefix("t.")?;
    let kind = rest
        .split(|character: char| character.is_whitespace() || character == '(')
        .next()?;
    let rest = rest[kind.len()..].trim_start();
    let (rest, quoted) = if let Some(rest) = rest.strip_prefix('"') {
        (rest, true)
    } else {
        (rest.strip_prefix(':')?, false)
    };
    let name = if quoted {
        rest.split('"').next()?
    } else {
        rest.split(|character: char| !(character.is_alphanumeric() || character == '_'))
            .next()?
    };
    let null = !line.contains("null: false");
    let types = column_type(kind, null);
    Some(Column {
        name: name.to_string(),
        types,
    })
}

fn column_type(kind: &str, null: bool) -> TypeExpr {
    let name = match kind {
        "string" | "text" | "binary" | "uuid" => "String",
        "integer" | "bigint" | "smallint" | "references" => "Integer",
        "float" | "decimal" => "Float",
        "datetime" | "time" | "timestamp" => "Time",
        "date" => "Date",
        "boolean" => {
            return optional(
                TypeExpr::Union(vec![
                    TypeExpr::Literal(SourceText::from("true")),
                    TypeExpr::Literal(SourceText::from("false")),
                ]),
                null,
            );
        }
        "json" | "jsonb" => {
            return optional(
                TypeExpr::Union(vec![
                    TypeExpr::Name(SourceText::from("Hash")),
                    TypeExpr::Name(SourceText::from("Array")),
                ]),
                null,
            );
        }
        _ => "Object",
    };
    optional(TypeExpr::Name(SourceText::from(name)), null)
}

fn optional(types: TypeExpr, null: bool) -> TypeExpr {
    if null {
        TypeExpr::Union(vec![types, TypeExpr::Name(SourceText::from("nil"))])
    } else {
        types
    }
}

fn model_name(registry: &Registry, namespace: ObjectId) -> String {
    registry
        .get(namespace)
        .name
        .to_string_lossy()
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .to_string()
}

fn snake_case(name: &str) -> String {
    let mut result = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                result.push('_');
            }
            result.push(character.to_ascii_lowercase());
        } else {
            result.push(character);
        }
    }
    result
}

fn pluralize(word: &str, irregulars: &BTreeMap<String, String>) -> String {
    if let Some(irregular) = irregulars
        .get(word)
        .map(String::as_str)
        .or_else(|| irregular_plural(word))
    {
        return irregular.to_string();
    }
    if word.ends_with('s') || word.ends_with('x') || word.ends_with("ch") || word.ends_with("sh") {
        format!("{word}es")
    } else if word.ends_with('y')
        && !word.ends_with("ay")
        && !word.ends_with("ey")
        && !word.ends_with("oy")
    {
        format!("{}ies", &word[..word.len() - 1])
    } else {
        format!("{word}s")
    }
}

fn singularize(word: &str, irregulars: &BTreeMap<String, String>) -> String {
    if let Some(irregular) = irregulars
        .iter()
        .find_map(|(singular, plural)| (plural == word).then_some(singular.as_str()))
        .or_else(|| irregular_singular(word))
    {
        return irregular.to_string();
    }
    if let Some(stem) = word.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = word
        .strip_suffix("ses")
        .or_else(|| word.strip_suffix("xes"))
        .or_else(|| word.strip_suffix("ches"))
        .or_else(|| word.strip_suffix("shes"))
    {
        stem.to_string()
    } else if let Some(stem) = word.strip_suffix('s').filter(|stem| !stem.ends_with('s')) {
        stem.to_string()
    } else {
        word.to_string()
    }
}

fn irregular_plural(word: &str) -> Option<&'static str> {
    match word {
        "person" => Some("people"),
        "man" => Some("men"),
        "woman" => Some("women"),
        "child" => Some("children"),
        "mouse" => Some("mice"),
        "datum" => Some("data"),
        _ => None,
    }
}

fn irregular_singular(word: &str) -> Option<&'static str> {
    match word {
        "people" => Some("person"),
        "men" => Some("man"),
        "women" => Some("woman"),
        "children" => Some("child"),
        "mice" => Some("mouse"),
        "data" => Some("datum"),
        _ => None,
    }
}

fn camelize(word: &str, irregulars: &BTreeMap<String, String>) -> String {
    let singular = singularize(word, irregulars);
    singular
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect()
}

fn append(name: &SourceText, suffix: &str) -> SourceText {
    let mut bytes = name.as_bytes().to_vec();
    bytes.extend_from_slice(suffix.as_bytes());
    SourceText::from_vec(bytes)
}

fn format_source(prefix: &str, name: &SourceText) -> SourceText {
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.extend_from_slice(name.as_bytes());
    SourceText::from_vec(bytes)
}
