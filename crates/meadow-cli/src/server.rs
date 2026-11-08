//! Development server with live reload and polling rebuilds.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use meadow_html::{SiteOptions, render_site};

use crate::cache::load_or_build;
use crate::config::RunOptions;
use crate::discover::discover;

type Subscribers = Arc<Mutex<Vec<Sender<()>>>>;

/// Serves the output directory and rebuilds when source files change.
///
/// # Errors
///
/// Returns an error when the listener cannot bind or the initial build fails.
pub fn serve(options: RunOptions) -> io::Result<()> {
    let options = Arc::new(options);
    rebuild(&options)?;

    let listener = TcpListener::bind(("127.0.0.1", options.port))?;
    let address = listener.local_addr()?;
    println!(
        "Serving {} at http://{address}",
        options.output_dir.display()
    );

    let subscribers: Subscribers = Arc::new(Mutex::new(Vec::new()));
    spawn_watcher(Arc::clone(&options), Arc::clone(&subscribers));

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let options = Arc::clone(&options);
        let subscribers = Arc::clone(&subscribers);
        thread::spawn(move || {
            let _ = handle(stream, &options.output_dir, &subscribers);
        });
    }
    Ok(())
}

fn rebuild(options: &RunOptions) -> io::Result<()> {
    let result = load_or_build(options)?;
    if options.fail_on_warning && !result.warnings.is_empty() {
        for warning in &result.warnings {
            eprintln!("warning: {}", warning.message);
        }
        return Err(io::Error::other("warnings treated as errors"));
    }
    if options.verbose > 0 && !options.quiet {
        eprintln!(
            "rebuilt registry: {} files ({} parsed, {} reused)",
            result.stats.files, result.stats.parsed, result.stats.reused
        );
    }
    render_site(
        &result.registry,
        &SiteOptions {
            output_dir: options.output_dir.clone(),
            title: options.title.clone(),
            readme: options.readme.clone(),
            main: options.main.clone(),
            files: options.files.clone(),
            template_path: None,
            project_root: std::env::current_dir()?,
            sanitize: false,
            markup: options.markup.clone(),
            show_private: options.show_private,
            show_protected: options.show_protected,
            assets: options.assets.clone(),
            api: options.api.clone(),
            hide_api: options.hide_api,
            show_inherited: options.show_inherited,
            embed_mixins: options.embed_mixins,
            timestamps: options.timestamps,
            proxy_urls: load_proxy_urls(options),
            cache_dir: options.use_cache.then(|| options.db.clone()),
        },
    )?;
    Ok(())
}

fn spawn_watcher(options: Arc<RunOptions>, subscribers: Subscribers) {
    thread::spawn(move || {
        let mut last = source_signature(&options);
        loop {
            thread::sleep(Duration::from_millis(500));
            let current = source_signature(&options);
            if current != last {
                thread::sleep(Duration::from_millis(200));
                last = source_signature(&options);
                if !options.quiet {
                    eprintln!("change detected; rebuilding...");
                }
                if let Err(error) = rebuild(&options) {
                    eprintln!("rebuild failed: {error}");
                } else {
                    let mut subscribers = subscribers.lock().expect("subscriber lock");
                    subscribers.retain(|subscriber| subscriber.send(()).is_ok());
                }
            }
        }
    });
}

fn source_signature(options: &RunOptions) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in discover(options) {
        path.hash(&mut hasher);
        if let Ok(metadata) = fs::metadata(&path) {
            metadata.len().hash(&mut hasher);
            if let Ok(modified) = metadata.modified() {
                modified
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos())
                    .hash(&mut hasher);
            }
        }
    }
    for config in [".yardopts", ".meadow.toml"] {
        if let Ok(metadata) = fs::metadata(config) {
            config.hash(&mut hasher);
            metadata.len().hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn handle(mut stream: TcpStream, output_dir: &Path, subscribers: &Subscribers) -> io::Result<()> {
    let mut buffer = [0_u8; 8192];
    let read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let Some(line) = request.lines().next() else {
        return Ok(());
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        return write_response(&mut stream, 405, "text/plain", b"method not allowed");
    }
    let path = path.split('?').next().unwrap_or(path);

    if path == "/__meadow_events" {
        return stream_events(stream, subscribers);
    }
    if path == "/livereload.js" {
        return write_response(
            &mut stream,
            200,
            "application/javascript",
            b"const es=new EventSource('/__meadow_events');es.onmessage=()=>location.reload();",
        );
    }

    let relative = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    if relative.contains("..") {
        return write_response(&mut stream, 403, "text/plain", b"forbidden");
    }
    let file = output_dir.join(relative);
    let Ok(bytes) = fs::read(&file) else {
        return write_response(&mut stream, 404, "text/plain", b"not found");
    };
    let content_type = content_type(&file);
    if content_type == "text/html; charset=utf-8" {
        let mut html = String::from_utf8_lossy(&bytes).into_owned();
        if let Some(index) = html.rfind("</body>") {
            html.insert_str(index, "<script src=\"livereload.js\"></script>");
        }
        write_response(&mut stream, 200, content_type, html.as_bytes())
    } else {
        write_response(&mut stream, 200, content_type, &bytes)
    }
}

fn stream_events(mut stream: TcpStream, subscribers: &Subscribers) -> io::Result<()> {
    let (sender, receiver) = channel();
    subscribers.lock().expect("subscriber lock").push(sender);
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
    )?;
    stream.write_all(b": connected\n\n")?;
    stream.flush()?;
    for () in receiver {
        if stream.write_all(b"data: reload\n\n").is_err() || stream.flush().is_err() {
            break;
        }
    }
    Ok(())
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn load_proxy_urls(options: &RunOptions) -> std::collections::BTreeMap<String, String> {
    let Some(path) = &options.proxy_cache else {
        return std::collections::BTreeMap::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return std::collections::BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}
