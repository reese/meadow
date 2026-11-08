//! End-to-end CLI tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn temp_directory(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "meadow-cli-{prefix}-{}-{stamp}-{sequence}",
        std::process::id()
    ))
}

fn project() -> PathBuf {
    let directory = temp_directory("project");
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("lib")).expect("create lib");
    fs::write(
        directory.join("lib/demo.rb"),
        "# A demo class.\nclass Demo\n  # Greets someone.\n  # @param [String] name the name\n  # @return [String] greeting\n  def greet(name)\n    \"hi #{name}\"\n  end\n\n  private\n\n  def secret; end\nend\n",
    )
    .expect("write fixture");
    fs::write(
        directory.join("lib/extra.rb"),
        "# Another class.\nclass Extra\n  def keep; end\nend\n",
    )
    .expect("write extra fixture");
    fs::write(
        directory.join("README.md"),
        "# Demo\n\nWelcome to **Demo**.\n",
    )
    .expect("write readme");
    fs::write(
        directory.join(".yardopts"),
        "--title \"Demo Docs\"\n--output-dir site\n# a comment\n--no-private\n",
    )
    .expect("write yardopts");
    directory
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_meadow"))
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run meadow")
}

#[test]
fn builds_documentation_and_reuses_the_cache() {
    let directory = project();

    let first = run(&directory, &["doc", "-v"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(directory.join("site/class-demo.html").is_file());
    assert!(directory.join("site/search.json").is_file());
    let index = fs::read_to_string(directory.join("site/index.html")).expect("read index");
    assert!(index.contains("Demo Docs"));
    assert!(first.stdout.windows(8).any(|window| window == b"2 parsed"));

    let second = run(&directory, &["doc", "-v"]);
    assert!(second.status.success());
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("0 parsed"), "{stdout}");
    assert!(stdout.contains("yes cache hit"), "{stdout}");

    fs::write(
        directory.join("lib/demo.rb"),
        "# A demo class.\nclass Demo\n  # Changed.\n  def greet(name); end\nend\n",
    )
    .expect("rewrite fixture");
    let third = run(&directory, &["doc", "-v"]);
    assert!(third.status.success());
    let stdout = String::from_utf8_lossy(&third.stdout);
    assert!(stdout.contains("1 parsed"), "{stdout}");
    assert!(stdout.contains("1 reused"), "{stdout}");
    assert!(stdout.contains("no cache hit"), "{stdout}");
    assert!(directory.join("site/class-extra.html").is_file());
}

#[test]
fn discovers_app_and_sig_rbs_by_default() {
    let directory = temp_directory("rbs");
    fs::create_dir_all(directory.join("app/models")).expect("create app");
    fs::create_dir_all(directory.join("sig")).expect("create sig");
    fs::write(
        directory.join("app/models/widget.rb"),
        "class Widget\n  def name; end\nend\n",
    )
    .expect("write widget");
    fs::write(
        directory.join("sig/widget.rbs"),
        "class Widget\n  def serial: () -> String\nend\n",
    )
    .expect("write rbs");

    let output = run(&directory, &["list", "--format", "json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let entries = value.as_array().expect("array");
    assert!(entries.iter().any(|entry| entry["path"] == "Widget#name"));
    assert!(entries.iter().any(|entry| entry["path"] == "Widget#serial"));
}

#[test]
fn lists_and_filters_objects() {
    let directory = project();
    let output = run(
        &directory,
        &[
            "list",
            "--format",
            "json",
            "--query",
            "object.type == :method && object.visibility == :public",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let entries = value.as_array().expect("array");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry["path"] == "Demo#greet"));
    assert!(entries.iter().any(|entry| entry["path"] == "Extra#keep"));

    let output = run(&directory, &["stats"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Classes:        2"), "{stdout}");
    assert!(stdout.contains("Documented:"), "{stdout}");
}

#[test]
fn follows_relative_requires_and_accepts_ruby_version() {
    let directory = temp_directory("requires");
    fs::create_dir_all(directory.join("lib")).expect("create lib");
    fs::write(
        directory.join("lib/main.rb"),
        "require_relative \"helper\"\nclass Main; end\n",
    )
    .expect("write main");
    fs::write(directory.join("lib/helper.rb"), "class Helper; end\n").expect("write helper");

    let output = run(
        &directory,
        &[
            "doc",
            "lib/main.rb",
            "--follow-requires",
            "--ruby-version",
            "3.3",
            "--no-cache",
            "--no-save",
            "--output-dir",
            "site",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.join("site/class-helper.html").is_file());
    assert!(directory.join("site/class-main.html").is_file());
}

#[test]
fn reports_syntax_diagnostics_and_fails_on_warning() {
    let directory = temp_directory("broken");
    fs::create_dir_all(directory.join("lib")).expect("create lib");
    fs::write(
        directory.join("lib/broken.rb"),
        "class Broken\n  def oops(\nend\n",
    )
    .expect("write broken fixture");

    let stats = run(&directory, &["stats"]);
    let stderr = String::from_utf8_lossy(&stats.stderr);
    assert!(stderr.contains("broken.rb"), "{stderr}");
    assert!(stderr.contains("error"), "{stderr}");

    let doc = run(
        &directory,
        &[
            "doc",
            "--no-cache",
            "--no-save",
            "--fail-on-warning",
            "--output-dir",
            "site",
        ],
    );
    assert!(!doc.status.success());
}

#[test]
fn respects_visibility_options() {
    let directory = project();
    let first = run(&directory, &["doc"]);
    assert!(first.status.success());
    let default_page =
        fs::read_to_string(directory.join("site/class-demo.html")).expect("default page");
    assert!(!default_page.contains("secret"));

    let second = run(
        &directory,
        &["doc", "--private", "--output-dir", "site-private"],
    );
    assert!(second.status.success());
    let private_page =
        fs::read_to_string(directory.join("site-private/class-demo.html")).expect("private page");
    assert!(private_page.contains("secret"));
}
