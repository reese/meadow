//! HTML templates, search index, and site writer for meadow.

#![deny(missing_docs)]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use meadow_markup::{Markdown, Markup, RDoc};
use meadow_model::{
    CodeObject, KindData, ObjectId, ObjectKind, Registry, Scope, SourceText, Tag, TypeExpr,
    Visibility,
};
use meadow_resolve::{LinkTargetResolver, expand_links, resolve_name};
use minijinja::Environment;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

static TEMPLATES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../templates");

/// Options controlling site generation.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct SiteOptions {
    /// Directory where pages are written.
    pub output_dir: PathBuf,
    /// Project title shown in the header and page titles.
    pub title: String,
    /// README or landing-page source.
    pub readme: Option<PathBuf>,
    /// Main page source, overriding the README on the index page.
    pub main: Option<PathBuf>,
    /// Additional documentation files.
    pub files: Vec<PathBuf>,
    /// Optional template override directory.
    pub template_path: Option<PathBuf>,
    /// Root used to resolve relative file paths.
    pub project_root: PathBuf,
    /// Whether raw HTML in Markdown is escaped.
    pub sanitize: bool,
    /// Default docstring markup provider (`markdown` or `rdoc`).
    pub markup: String,
    /// Include private methods and attributes.
    pub show_private: bool,
    /// Include protected methods and attributes.
    pub show_protected: bool,
    /// Additional asset mappings from source to destination.
    pub assets: Vec<(PathBuf, PathBuf)>,
    /// API tags selected by the user.
    pub api: Vec<String>,
    /// Whether the selected API tags should be hidden instead of shown.
    pub hide_api: bool,
    /// Show inherited methods.
    pub show_inherited: bool,
    /// Embed mixin methods directly in namespace groups.
    pub embed_mixins: bool,
    /// Emit generation timestamps.
    pub timestamps: bool,
    /// External proxy name-to-URL mappings.
    pub proxy_urls: BTreeMap<String, String>,
    /// Optional page-cache directory for incremental rendering.
    pub cache_dir: Option<PathBuf>,
}

impl Default for SiteOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("doc"),
            title: "Documentation".to_string(),
            readme: None,
            main: None,
            files: Vec::new(),
            template_path: None,
            project_root: PathBuf::from("."),
            sanitize: false,
            markup: "markdown".to_string(),
            show_private: false,
            show_protected: true,
            assets: Vec::new(),
            api: Vec::new(),
            hide_api: false,
            show_inherited: true,
            embed_mixins: false,
            timestamps: false,
            proxy_urls: BTreeMap::new(),
            cache_dir: None,
        }
    }
}

/// Summary of a completed site build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteStats {
    /// Number of HTML pages written.
    pub pages: usize,
    /// Number of search-index entries.
    pub search_entries: usize,
    /// Link/include warnings produced while rendering.
    pub warnings: Vec<String>,
    /// Pages actually written.
    pub pages_written: usize,
    /// Pages whose content was unchanged.
    pub pages_unchanged: usize,
}

struct SiteResolver {
    project_root: PathBuf,
    proxy_urls: BTreeMap<String, String>,
}

impl LinkTargetResolver for SiteResolver {
    fn object_url(&self, registry: &Registry, id: ObjectId) -> String {
        object_url(registry, id)
    }

    fn file_url(&self, path: &str) -> String {
        format!("file-{}.html", slug(path))
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(self.project_root.join(path)).ok()
    }

    fn proxy_url(&self, name: &str) -> Option<String> {
        self.proxy_urls.get(name).cloned()
    }
}

/// Renders the full documentation site.
///
/// # Errors
///
/// Returns an I/O or template error when output cannot be written.
pub fn render_site(registry: &Registry, options: &SiteOptions) -> io::Result<SiteStats> {
    fs::create_dir_all(&options.output_dir)?;
    let mut environment = environment(options.template_path.as_deref())?;
    if options.timestamps {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        environment.add_global(
            "timestamp",
            minijinja::Value::from(format!("Unix timestamp {seconds}")),
        );
    }
    let resolver = SiteResolver {
        project_root: options.project_root.clone(),
        proxy_urls: options.proxy_urls.clone(),
    };
    let digest = template_digest(options);
    let mut writer = PageWriter::new(&environment, options, digest);

    let classes = class_list(registry);
    let index_content = index_content(options);
    let overview_context = json!({
        "site_title": options.title,
        "title": "Overview",
        "classes": classes,
        "content_html": index_content,
    });
    writer.write("index.html", "index.html", overview_context)?;

    let warnings = write_class_pages(registry, &resolver, options, &mut writer)?;

    let class_list_context = json!({
        "site_title": options.title,
        "title": "Classes",
        "classes": classes,
    });
    writer.write("class_list.html", "class_list.html", class_list_context)?;

    let methods = all_methods(registry, &resolver, options);
    let method_context = json!({
        "site_title": options.title,
        "title": "Methods",
        "methods": methods,
    });
    writer.write("method_list.html", "method_list.html", method_context)?;

    let top_level_context = json!({
        "site_title": options.title,
        "title": "Top Level Namespace",
        "objects": registry
            .children(ObjectId::ROOT)
            .iter()
            .map(|id| {
                let object = registry.get(*id);
                json!({
                    "path": object.path,
                    "kind": kind_name(object.kind),
                    "url": object_url(registry, *id),
                    "anchor": object_anchor(object),
                })
            })
            .collect::<Vec<_>>(),
    });
    writer.write("top_level.html", "top_level.html", top_level_context)?;

    let files = file_entries(options);
    write_file_pages(&files, options, &mut writer)?;

    let undocumented_context = undocumented_context(registry, options);
    writer.write(
        "undocumented.html",
        "undocumented.html",
        undocumented_context,
    )?;

    let search = search_entries(registry, &files, options);
    write_if_changed(
        &options.output_dir.join("search.json"),
        &serde_json::to_vec_pretty(&search)?,
    )?;
    write_assets(options)?;
    copy_assets(options)?;

    writer.finish()?;
    Ok(SiteStats {
        pages: writer.written + writer.unchanged,
        search_entries: search.len(),
        warnings,
        pages_written: writer.written,
        pages_unchanged: writer.unchanged,
    })
}

fn write_file_pages(
    files: &[Value],
    options: &SiteOptions,
    writer: &mut PageWriter<'_>,
) -> io::Result<()> {
    for file in files {
        let context = json!({
            "site_title": options.title,
            "title": file["name"],
            "file": file,
        });
        let name = file["url"].as_str().unwrap_or("file.html");
        writer.write("file.html", name, context)?;
    }
    let file_context = json!({
        "site_title": options.title,
        "title": "Files",
        "files": files,
    });
    writer.write("file_list.html", "file_list.html", file_context)
}

fn write_class_pages(
    registry: &Registry,
    resolver: &SiteResolver,
    options: &SiteOptions,
    writer: &mut PageWriter<'_>,
) -> io::Result<Vec<String>> {
    let ids = namespace_ids(registry);
    let mut warnings = Vec::new();
    for chunk in ids.chunks(PAGE_BATCH) {
        let class_pages: Vec<(String, Value, Vec<String>)> = chunk
            .par_iter()
            .map(|id| {
                let mut page_warnings = Vec::new();
                let context = class_context(registry, resolver, *id, options, &mut page_warnings);
                let file_name = format!(
                    "class-{}.html",
                    slug(&registry.get(*id).path.to_string_lossy())
                );
                (file_name, context, page_warnings)
            })
            .collect();
        for (file_name, context, page_warnings) in class_pages {
            warnings.extend(page_warnings);
            writer.write("class.html", &file_name, context)?;
        }
    }
    Ok(warnings)
}

fn undocumented_context(registry: &Registry, options: &SiteOptions) -> Value {
    let mut undocumented: Vec<&CodeObject> = registry
        .objects()
        .iter()
        .filter(|object| {
            !matches!(object.kind, ObjectKind::Root | ObjectKind::Macro)
                && object.docstring.text.is_empty()
                && object.docstring.tags.is_empty()
        })
        .collect();
    undocumented.sort_by(|left, right| left.path.cmp(&right.path));
    let undocumented: Vec<Value> = undocumented
        .into_iter()
        .map(|object| {
            json!({
                "path": object.path,
                "kind": kind_name(object.kind),
                "url": object_url(registry, object.id),
            })
        })
        .collect();
    json!({
        "site_title": options.title,
        "title": "Undocumented objects",
        "objects": undocumented,
    })
}

fn copy_assets(options: &SiteOptions) -> io::Result<()> {
    for (source, destination) in &options.assets {
        let target = options.output_dir.join(destination);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, &target)?;
    }
    Ok(())
}

const PAGE_BATCH: usize = 64;

struct PageJob {
    template: String,
    file: String,
    context: Value,
    hash: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PageState {
    context: u64,
    content: u64,
}

struct PageWriter<'a> {
    environment: &'a Environment<'static>,
    output_dir: &'a Path,
    digest: u64,
    hashes: BTreeMap<String, PageState>,
    written: usize,
    unchanged: usize,
    cache_dir: Option<&'a Path>,
    pending: Vec<PageJob>,
}

impl<'a> PageWriter<'a> {
    fn new(environment: &'a Environment<'static>, options: &'a SiteOptions, digest: u64) -> Self {
        Self {
            environment,
            output_dir: &options.output_dir,
            digest,
            hashes: if options.timestamps {
                BTreeMap::new()
            } else {
                load_page_hashes(options.cache_dir.as_deref())
            },
            written: 0,
            unchanged: 0,
            cache_dir: options.cache_dir.as_deref(),
            pending: Vec::new(),
        }
    }

    fn write(&mut self, template_name: &str, file_name: &str, context: Value) -> io::Result<()> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.digest.hash(&mut hasher);
        serde_json::to_vec(&context)?.hash(&mut hasher);
        let hash = hasher.finish();
        let path = self.output_dir.join(file_name);
        if let Some(state) = self.hashes.get(file_name)
            && state.context == hash
            && file_hash(&path) == Some(state.content)
        {
            self.unchanged += 1;
            return Ok(());
        }
        self.pending.push(PageJob {
            template: template_name.to_string(),
            file: file_name.to_string(),
            context,
            hash,
        });
        if self.pending.len() >= PAGE_BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let jobs = std::mem::take(&mut self.pending);
        let output_dir = self.output_dir;
        let environment = self.environment;
        let results: io::Result<Vec<(String, PageState)>> = jobs
            .par_iter()
            .map(|job| {
                let template = environment
                    .get_template(&job.template)
                    .map_err(io::Error::other)?;
                let html = template
                    .render(json_to_value(&job.context))
                    .map_err(io::Error::other)?;
                write_if_changed(&output_dir.join(&job.file), html.as_bytes())?;
                Ok((
                    job.file.clone(),
                    PageState {
                        context: job.hash,
                        content: content_hash(html.as_bytes()),
                    },
                ))
            })
            .collect();
        let results = results?;
        self.written += results.len();
        for (file, state) in results {
            self.hashes.insert(file, state);
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.flush()?;
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let Some(directory) = self.cache_dir else {
            return Ok(());
        };
        fs::create_dir_all(directory)?;
        fs::write(
            directory.join("pages.json"),
            serde_json::to_vec_pretty(&self.hashes).map_err(io::Error::other)?,
        )
    }
}

fn load_page_hashes(cache_dir: Option<&Path>) -> BTreeMap<String, PageState> {
    let Some(directory) = cache_dir else {
        return BTreeMap::new();
    };
    fs::read(directory.join("pages.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn content_hash(contents: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    contents.hash(&mut hasher);
    hasher.finish()
}

fn file_hash(path: &Path) -> Option<u64> {
    fs::read(path).ok().map(|contents| content_hash(&contents))
}

fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Ok(metadata) = fs::metadata(path)
        && metadata.len() == u64::try_from(contents.len()).unwrap_or(u64::MAX)
        && fs::read(path).is_ok_and(|existing| existing == contents)
    {
        return Ok(());
    }
    fs::write(path, contents)
}

fn template_digest(options: &SiteOptions) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    options.markup.hash(&mut hasher);
    options.sanitize.hash(&mut hasher);
    options.show_private.hash(&mut hasher);
    options.show_protected.hash(&mut hasher);
    options.show_inherited.hash(&mut hasher);
    options.embed_mixins.hash(&mut hasher);
    options.api.hash(&mut hasher);
    options.hide_api.hash(&mut hasher);
    options.title.hash(&mut hasher);
    options.timestamps.hash(&mut hasher);
    for file in TEMPLATES.files() {
        file.path().hash(&mut hasher);
        file.contents().hash(&mut hasher);
    }
    if let Some(directory) = &options.template_path {
        for asset in [
            "base.html",
            "index.html",
            "class.html",
            "class_list.html",
            "method_list.html",
            "file_list.html",
            "file.html",
            "undocumented.html",
            "top_level.html",
            "style.css",
            "search.js",
        ] {
            if let Ok(contents) = fs::read(directory.join(asset)) {
                asset.hash(&mut hasher);
                contents.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn json_to_value(value: &Value) -> minijinja::Value {
    match value {
        Value::Null => minijinja::Value::from(()),
        Value::Bool(value) => minijinja::Value::from(*value),
        Value::Number(value) => value.as_i64().map_or_else(
            || {
                value.as_f64().map_or_else(
                    || minijinja::Value::from(value.to_string()),
                    minijinja::Value::from,
                )
            },
            minijinja::Value::from,
        ),
        Value::String(value) => minijinja::Value::from(value.clone()),
        Value::Array(values) => {
            minijinja::Value::from(values.iter().map(json_to_value).collect::<Vec<_>>())
        }
        Value::Object(values) => minijinja::Value::from_pairs(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_value(value))),
        ),
    }
}

fn environment(override_dir: Option<&Path>) -> io::Result<Environment<'static>> {
    let mut environment = Environment::new();
    for file in TEMPLATES.files() {
        let Some(name) = file.path().file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let mut content = String::from_utf8_lossy(file.contents()).into_owned();
        if let Some(directory) = override_dir {
            let path = directory.join(name);
            if path.is_file() {
                content = fs::read_to_string(path)?;
            }
        }
        environment
            .add_template_owned(name.to_string(), content)
            .map_err(io::Error::other)?;
    }
    Ok(environment)
}

fn write_assets(options: &SiteOptions) -> io::Result<()> {
    for asset in ["style.css", "search.js"] {
        let mut contents = TEMPLATES
            .get_file(asset)
            .map_or_else(Vec::new, |file| file.contents().to_vec());
        if let Some(directory) = &options.template_path {
            let path = directory.join(asset);
            if path.is_file() {
                contents = fs::read(path)?;
            }
        }
        fs::write(options.output_dir.join(asset), contents)?;
    }
    Ok(())
}

fn namespace_ids(registry: &Registry) -> Vec<ObjectId> {
    registry
        .objects()
        .iter()
        .filter(|object| matches!(object.kind, ObjectKind::Class | ObjectKind::Module))
        .map(|object| object.id)
        .collect()
}

fn class_list(registry: &Registry) -> Vec<Value> {
    let mut ids = namespace_ids(registry);
    ids.sort_by(|left, right| registry.get(*left).path.cmp(&registry.get(*right).path));
    ids.into_iter()
        .map(|id| {
            let object = registry.get(id);
            json!({
                "name": object.name,
                "path": object.path,
                "kind": kind_name(object.kind),
                "url": object_url(registry, id),
                "summary": object.docstring.summary,
            })
        })
        .collect()
}

fn class_context(
    registry: &Registry,
    resolver: &SiteResolver,
    id: ObjectId,
    options: &SiteOptions,
    warnings: &mut Vec<String>,
) -> Value {
    let object = registry.get(id);
    let (superclass, ancestors, includes, extends, prepends) = match &object.extra {
        KindData::Namespace {
            superclass,
            ancestors,
            includes,
            extends,
            prepends,
            ..
        } => (
            superclass.clone(),
            ancestors.clone(),
            includes.clone(),
            extends.clone(),
            prepends.clone(),
        ),
        _ => (None, Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };

    let children: Vec<&CodeObject> = registry
        .children(id)
        .iter()
        .map(|child| registry.get(*child))
        .collect();
    let (constants, class_vars) = constant_views(registry, &children);

    let attributes = attribute_views(registry, &children);
    let own_methods: Vec<&CodeObject> = children
        .iter()
        .copied()
        .filter(|child| child.kind == ObjectKind::Method && visible(child, options))
        .collect();
    let mut seen = HashSet::new();
    for method in &own_methods {
        if let KindData::Method { scope, .. } = &method.extra {
            seen.insert((*scope, method.name.clone()));
        }
    }

    let mut methods: Vec<Value> = own_methods
        .iter()
        .map(|method| method_view(registry, resolver, method, None, options, warnings))
        .collect();

    methods.extend(inherited_method_views(
        registry, resolver, object, &seen, options, warnings,
    ));

    let groups = group_methods(methods);

    json!({
        "site_title": options.title,
        "title": object.path,
        "object": {
            "name": object.name,
            "path": object.path,
            "kind": kind_name(object.kind),
            "summary": object.docstring.summary,
            "html_doc": render_doc(
                registry,
                resolver,
                id,
                &object.docstring.text,
                options,
                docstring_markup(&object.docstring).as_deref(),
                warnings,
            ),
        },
        "superclass": superclass.map(|reference| ref_view(registry, resolver, reference)),
        "ancestors": ancestors.iter().map(|ancestor| view_for_id(registry, *ancestor)).collect::<Vec<_>>(),
        "includes": includes.iter().map(|reference| ref_view(registry, resolver, reference.clone())).collect::<Vec<_>>(),
        "extends": extends.iter().map(|reference| ref_view(registry, resolver, reference.clone())).collect::<Vec<_>>(),
        "prepends": prepends.iter().map(|reference| ref_view(registry, resolver, reference.clone())).collect::<Vec<_>>(),
        "constants": constants,
        "class_vars": class_vars,
        "attributes": attributes,
        "groups": groups,
    })
}

fn constant_views(registry: &Registry, children: &[&CodeObject]) -> (Vec<Value>, Vec<Value>) {
    let constants = children
        .iter()
        .filter(|child| child.kind == ObjectKind::Constant)
        .map(|child| {
            let value = match &child.extra {
                KindData::Constant { value_source, .. } => {
                    value_source.to_string_lossy().into_owned()
                }
                _ => String::new(),
            };
            json!({
                "name": child.name,
                "url": object_url(registry, child.id),
                "anchor": slug(&child.name.to_string_lossy()),
                "value": value,
            })
        })
        .collect();
    let class_vars = children
        .iter()
        .filter(|child| child.kind == ObjectKind::ClassVariable)
        .map(|child| json!({"name": child.name, "url": object_url(registry, child.id)}))
        .collect();
    (constants, class_vars)
}

fn inherited_method_views(
    registry: &Registry,
    resolver: &SiteResolver,
    object: &CodeObject,
    seen: &HashSet<(Scope, SourceText)>,
    options: &SiteOptions,
    warnings: &mut Vec<String>,
) -> Vec<Value> {
    let KindData::Namespace {
        ancestors,
        includes,
        extends,
        prepends,
        ..
    } = &object.extra
    else {
        return Vec::new();
    };
    let mut mixin_ids = HashSet::new();
    if !options.embed_mixins {
        for reference in includes.iter().chain(extends.iter()).chain(prepends.iter()) {
            if let meadow_model::Ref::Resolved(mixin) = reference {
                collect_mixin(registry, *mixin, &mut mixin_ids);
            }
        }
    }

    let mut methods = Vec::new();
    if !options.show_inherited {
        return methods;
    }
    let mut inherited_seen = HashSet::new();
    for ancestor in ancestors {
        if !options.embed_mixins && mixin_ids.contains(ancestor) {
            continue;
        }
        for child in registry.children(*ancestor) {
            let method = registry.get(*child);
            if method.kind != ObjectKind::Method || !visible(method, options) {
                continue;
            }
            let KindData::Method { scope, .. } = &method.extra else {
                continue;
            };
            if seen.contains(&(*scope, method.name.clone()))
                || !inherited_seen.insert((*scope, method.name.clone()))
            {
                continue;
            }
            let parent = registry.get(*ancestor);
            methods.push(method_view(
                registry,
                resolver,
                method,
                Some((&parent.path, object_url(registry, parent.id))),
                options,
                warnings,
            ));
        }
    }
    methods
}

fn group_methods(methods: Vec<Value>) -> Vec<Value> {
    let mut groups: BTreeMap<String, (Vec<Value>, Vec<Value>)> = BTreeMap::new();
    for method in methods {
        let kind = method["scope"].as_str().unwrap_or("instance");
        let group = method["group"].as_str().unwrap_or("Methods").to_string();
        let entry = groups.entry(group).or_default();
        if kind == "class" {
            entry.1.push(method);
        } else {
            entry.0.push(method);
        }
    }
    groups
        .into_iter()
        .map(|(name, (instance, class))| {
            json!({"name": name, "instance_methods": instance, "class_methods": class})
        })
        .collect()
}

fn attribute_views(registry: &Registry, children: &[&CodeObject]) -> Vec<Value> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for child in children {
        let KindData::Method {
            attr: Some(attr), ..
        } = &child.extra
        else {
            continue;
        };
        if !seen.insert(attr.name.clone()) {
            continue;
        }
        let url = object_url(registry, child.id);
        result.push(json!({
            "name": attr.name,
            "reader": attr.reader,
            "writer": attr.writer,
            "url": url,
        }));
    }
    result
}

fn collect_mixin(registry: &Registry, namespace: ObjectId, ids: &mut HashSet<ObjectId>) {
    if !ids.insert(namespace) {
        return;
    }
    if let KindData::Namespace { ancestors, .. } = &registry.get(namespace).extra {
        for ancestor in ancestors {
            collect_mixin(registry, *ancestor, ids);
        }
    }
}

fn method_view(
    registry: &Registry,
    resolver: &SiteResolver,
    object: &CodeObject,
    inherited_from: Option<(&SourceText, String)>,
    options: &SiteOptions,
    warnings: &mut Vec<String>,
) -> Value {
    let (scope, signature, source, attr, aliases) = match &object.extra {
        KindData::Method {
            scope,
            signature,
            source,
            attr,
            aliases,
            ..
        } => (*scope, signature, source, attr.clone(), aliases),
        _ => return Value::Null,
    };
    let namespace = object.namespace.unwrap_or(ObjectId::ROOT);
    let ally = aliases
        .iter()
        .map(|id| {
            let alias = registry.get(*id);
            json!({"name": alias.name, "url": object_url(registry, *id)})
        })
        .collect::<Vec<_>>();

    json!({
        "name": object.name,
        "path": object.path,
        "url": method_url(registry, object),
        "anchor": anchor_for_method(object),
        "scope": scope_name(scope),
        "visibility": visibility_name(object.visibility),
        "signature": signature,
        "source": escape_html(&source.to_string_lossy()),
        "summary": object.docstring.summary,
        "html_doc": render_doc(
            registry,
            resolver,
            namespace,
            &object.docstring.text,
            options,
            docstring_markup(&object.docstring).as_deref(),
            warnings,
        ),
        "tags_html": render_tags(
            registry,
            resolver,
            namespace,
            &object.docstring.tags,
            options,
            warnings,
        ),
        "aliases": ally,
        "group": object.group,
        "attr": attr.map(|attr| json!({"name": attr.name, "reader": attr.reader, "writer": attr.writer})),
        "inherited_from": inherited_from.map(|(name, url)| json!({"name": name, "url": url})),
    })
}

fn all_methods(registry: &Registry, resolver: &SiteResolver, options: &SiteOptions) -> Vec<Value> {
    let mut methods: Vec<&CodeObject> = registry
        .objects()
        .iter()
        .filter(|object| object.kind == ObjectKind::Method && visible(object, options))
        .collect();
    methods.sort_by(|left, right| left.path.cmp(&right.path));
    methods
        .into_iter()
        .map(|method| {
            let owner = method
                .namespace
                .map_or_else(|| registry.root().clone(), |id| registry.get(id).clone());
            let KindData::Method { scope, .. } = &method.extra else {
                return Value::Null;
            };
            let _ = resolver;
            json!({
                "name": method.name,
                "path": method.path,
                "url": method_url(registry, method),
                "scope": scope_name(*scope),
                "owner_name": owner.path,
                "owner_url": object_url(registry, owner.id),
            })
        })
        .collect()
}

fn render_doc(
    registry: &Registry,
    resolver: &SiteResolver,
    namespace: ObjectId,
    text: &SourceText,
    options: &SiteOptions,
    markup_override: Option<&str>,
    warnings: &mut Vec<String>,
) -> String {
    let expanded = expand_links(text, namespace, registry, resolver);
    warnings.extend(expanded.warnings.iter().cloned());
    let provider = markup_provider(options, markup_override);
    provider.render(&expanded.text.to_string_lossy())
}

fn markup_provider(options: &SiteOptions, markup_override: Option<&str>) -> Box<dyn Markup> {
    match markup_override.unwrap_or(&options.markup) {
        "rdoc" => Box::new(RDoc::new(options.sanitize)),
        _ => Box::new(Markdown::new(options.sanitize)),
    }
}

fn docstring_markup(docstring: &meadow_model::Docstring) -> Option<String> {
    docstring.tags.iter().find_map(|tag| {
        (tag.tag.as_bytes() == b"markup").then(|| {
            tag.text
                .to_string_lossy()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
    })
}

fn render_tags(
    registry: &Registry,
    resolver: &SiteResolver,
    namespace: ObjectId,
    tags: &[Tag],
    options: &SiteOptions,
    warnings: &mut Vec<String>,
) -> String {
    let mut result = String::new();
    for tag in tags {
        if tag.tag.as_bytes() == b"markup" {
            continue;
        }
        let name = escape_html(&tag.tag.to_string_lossy());
        result.push_str("<div class=\"tag tag-");
        result.push_str(&slug(&tag.tag.to_string_lossy()));
        result.push_str("\"><span class=\"tag-name\">@");
        result.push_str(&name);
        result.push_str("</span>");
        if let Some(types) = &tag.types {
            result.push_str(" <span class=\"tag-types\">");
            result.push_str(&render_type(registry, resolver, namespace, types));
            result.push_str("</span>");
        } else if let Some(types) = &tag.types_source {
            result.push_str(" <span class=\"tag-types\">");
            result.push_str(&escape_html(&types.to_string_lossy()));
            result.push_str("</span>");
        }
        if let Some(tag_name) = &tag.name {
            result.push_str(" <span class=\"tag-param\">");
            result.push_str(&escape_html(&tag_name.to_string_lossy()));
            result.push_str("</span>");
        }
        if let Some(key) = &tag.key {
            result.push_str(" <span class=\"tag-key\">");
            result.push_str(&escape_html(&key.to_string_lossy()));
            result.push_str("</span>");
        }
        if let Some(default) = &tag.default {
            result.push_str(" <span class=\"tag-default\">(");
            result.push_str(&escape_html(&default.to_string_lossy()));
            result.push_str(")</span>");
        }
        if !tag.text.is_empty() {
            result.push_str(" <span class=\"tag-text\">");
            result.push_str(&escape_html(&tag.text.to_string_lossy()));
            result.push_str("</span>");
        }
        if let Some(docstring) = &tag.docstring {
            result.push_str("<div class=\"tag-doc\">");
            result.push_str(&render_doc(
                registry,
                resolver,
                namespace,
                &docstring.text,
                options,
                docstring_markup(docstring).as_deref(),
                warnings,
            ));
            result.push_str("</div>");
        }
        result.push_str("</div>");
    }
    result
}

fn render_type(
    registry: &Registry,
    resolver: &SiteResolver,
    namespace: ObjectId,
    expression: &TypeExpr,
) -> String {
    match expression {
        TypeExpr::Name(name) => resolve_name(registry, name, Some(namespace)).map_or_else(
            || {
                let text = name.to_string_lossy();
                resolver.proxy_url(&text).map_or_else(
                    || escape_html(&text),
                    |url| {
                        format!(
                            "<a href=\"{}\">{}</a>",
                            escape_html(&url),
                            escape_html(&text)
                        )
                    },
                )
            },
            |id| {
                format!(
                    "<a href=\"{}\">{}</a>",
                    escape_html(&object_url(registry, id)),
                    escape_html(&name.to_string_lossy())
                )
            },
        ),
        TypeExpr::Literal(name) | TypeExpr::Duck(name) => escape_html(&name.to_string_lossy()),
        TypeExpr::Generic { base, params } => {
            let params = params
                .iter()
                .map(|param| render_type(registry, resolver, namespace, param))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}&lt;{}&gt;",
                render_type(registry, resolver, namespace, base),
                params
            )
        }
        TypeExpr::Hash { key, value, style } => match style {
            meadow_model::HashStyle::Angles => format!(
                "Hash&lt;{}, {}&gt;",
                render_type(registry, resolver, namespace, key),
                render_type(registry, resolver, namespace, value)
            ),
            meadow_model::HashStyle::Braces => format!(
                "Hash{{{}=&gt;{}}}",
                render_type(registry, resolver, namespace, key),
                render_type(registry, resolver, namespace, value)
            ),
        },
        TypeExpr::Tuple(types) => format!(
            "({})",
            types
                .iter()
                .map(|ty| render_type(registry, resolver, namespace, ty))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeExpr::Union(types) => types
            .iter()
            .map(|ty| render_type(registry, resolver, namespace, ty))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn ref_view(registry: &Registry, resolver: &SiteResolver, reference: meadow_model::Ref) -> Value {
    match reference {
        meadow_model::Ref::Resolved(id) => view_for_id(registry, id),
        meadow_model::Ref::Proxy { name, .. } => {
            let url = resolver.proxy_url(&name.to_string_lossy());
            json!({"name": name, "url": url})
        }
    }
}

fn view_for_id(registry: &Registry, id: ObjectId) -> Value {
    let object = registry.get(id);
    json!({"name": object.path, "url": object_url(registry, id)})
}

fn file_entries(options: &SiteOptions) -> Vec<Value> {
    let mut paths = Vec::new();
    if let Some(path) = &options.main {
        paths.push(path.clone());
    }
    if let Some(path) = &options.readme {
        paths.push(path.clone());
    }
    paths.extend(options.files.iter().cloned());
    if paths.is_empty() {
        let readme = options.project_root.join("README.md");
        if readme.is_file() {
            paths.push(PathBuf::from("README.md"));
        }
    }

    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for path in paths {
        let display = path.to_string_lossy().into_owned();
        let url = format!("file-{}.html", slug(&display));
        if !seen.insert(url.clone()) {
            continue;
        }
        let full = if path.is_absolute() {
            path.clone()
        } else {
            options.project_root.join(&path)
        };
        let contents = fs::read_to_string(&full).unwrap_or_default();
        let html = render_file(options, &full, &contents);
        let name = full.file_name().map_or_else(
            || display.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        result.push(json!({
            "name": name,
            "path": display,
            "url": url,
            "html": html,
        }));
    }
    result
}

fn index_content(options: &SiteOptions) -> Option<String> {
    let candidate = options
        .main
        .as_ref()
        .or(options.readme.as_ref())
        .cloned()
        .or_else(|| {
            let readme = options.project_root.join("README.md");
            readme.is_file().then(|| PathBuf::from("README.md"))
        });
    let path = candidate?;
    let full = if path.is_absolute() {
        path
    } else {
        options.project_root.join(path)
    };
    let Ok(contents) = fs::read_to_string(&full) else {
        return None;
    };
    Some(render_file(options, &full, &contents))
}

fn search_entries(registry: &Registry, files: &[Value], options: &SiteOptions) -> Vec<Value> {
    let mut entries: Vec<Value> = registry
        .objects()
        .iter()
        .filter(|object| {
            matches!(
                object.kind,
                ObjectKind::Class
                    | ObjectKind::Module
                    | ObjectKind::Method
                    | ObjectKind::Constant
                    | ObjectKind::ClassVariable
            ) && visible(object, options)
        })
        .map(|object| {
            json!({
                "name": object.name,
                "path": object.path,
                "kind": kind_name(object.kind),
                "url": object_url(registry, object.id),
                "summary": object.docstring.summary,
            })
        })
        .collect();

    for file in files {
        entries.push(json!({
            "name": file["name"],
            "path": file["path"],
            "kind": "file",
            "url": file["url"],
            "summary": "",
        }));
    }
    entries.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    entries
}

fn visible(object: &CodeObject, options: &SiteOptions) -> bool {
    let visibility = match object.visibility {
        Visibility::Private => options.show_private,
        Visibility::Protected => options.show_protected,
        Visibility::Public => true,
    };
    if !visibility || options.api.is_empty() {
        return visibility;
    }

    let api = object
        .docstring
        .tags
        .iter()
        .find(|tag| tag.tag == "api")
        .map(|tag| tag.text.to_string_lossy().into_owned());
    if options.hide_api {
        api.is_none_or(|api| !options.api.contains(&api))
    } else {
        api.is_some_and(|api| options.api.contains(&api))
    }
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Root => "root",
        ObjectKind::Module => "module",
        ObjectKind::Class => "class",
        ObjectKind::Method => "method",
        ObjectKind::Constant => "constant",
        ObjectKind::ClassVariable => "class_variable",
        ObjectKind::InstanceVariable => "instance_variable",
        ObjectKind::Macro => "macro",
    }
}

fn scope_name(scope: meadow_model::Scope) -> &'static str {
    match scope {
        meadow_model::Scope::Instance => "instance",
        meadow_model::Scope::Class => "class",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
    }
}

fn object_url(registry: &Registry, id: ObjectId) -> String {
    let object = registry.get(id);
    match object.kind {
        ObjectKind::Root => "index.html".to_string(),
        ObjectKind::Class | ObjectKind::Module => {
            format!("class-{}.html", slug(&object.path.to_string_lossy()))
        }
        ObjectKind::Method => method_url(registry, object),
        ObjectKind::Constant
        | ObjectKind::ClassVariable
        | ObjectKind::InstanceVariable
        | ObjectKind::Macro => {
            let anchor = object_anchor(object);
            match object.namespace {
                Some(owner) if owner != ObjectId::ROOT => format!(
                    "class-{}.html#{anchor}",
                    slug(&registry.get(owner).path.to_string_lossy())
                ),
                _ => format!("top_level.html#{anchor}"),
            }
        }
    }
}

fn method_url(registry: &Registry, object: &CodeObject) -> String {
    let anchor = anchor_for_method(object);
    match object.namespace {
        Some(owner) if owner != ObjectId::ROOT => format!(
            "class-{}.html#{anchor}",
            slug(&registry.get(owner).path.to_string_lossy())
        ),
        _ => format!("top_level.html#{anchor}"),
    }
}

fn object_anchor(object: &CodeObject) -> String {
    match object.kind {
        ObjectKind::Class | ObjectKind::Module => {
            format!("namespace-{}", slug(&object.path.to_string_lossy()))
        }
        ObjectKind::Method => anchor_for_method(object),
        ObjectKind::Constant => {
            format!("constant-{}", slug(&object.name.to_string_lossy()))
        }
        ObjectKind::ClassVariable | ObjectKind::InstanceVariable | ObjectKind::Macro => {
            format!("variable-{}", slug(&object.name.to_string_lossy()))
        }
        ObjectKind::Root => "top".to_string(),
    }
}

fn anchor_for_method(object: &CodeObject) -> String {
    let scope = match &object.extra {
        KindData::Method { scope, .. } => scope_name(*scope),
        _ => "instance",
    };
    format!("method-{scope}-{}", slug(&object.name.to_string_lossy()))
}

fn slug(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut last_dash = true;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            result.push('-');
            last_dash = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        "index".to_string()
    } else {
        result
    }
}

fn escape_html(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&#39;"),
            _ => result.push(character),
        }
    }
    result
}

fn render_file(options: &SiteOptions, path: &Path, contents: &str) -> String {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md" | "markdown") => Markdown::new(options.sanitize).render(contents),
        Some("rdoc") => RDoc::new(options.sanitize).render(contents),
        _ if options.markup == "rdoc" => RDoc::new(options.sanitize).render(contents),
        _ => Markdown::new(options.sanitize).render(contents),
    }
}
