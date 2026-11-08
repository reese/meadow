//! Synthetic large-corpus smoke test and timing baseline.

use std::fmt::Write;
use std::time::Instant;

use meadow_handlers::build_registry;
use meadow_parser::{FileFacts, parse_source};

#[test]
fn builds_a_large_synthetic_registry() {
    let mut facts: Vec<FileFacts> = Vec::new();
    for file in 0..200 {
        let mut source = String::new();
        for class in 0..3 {
            let _ = writeln!(
                source,
                "# Class {file}-{class}.\nclass Generated{file}_{class}"
            );
            let _ = writeln!(source, "  CONST_{class} = {class}");
            for method in 0..20 {
                let _ = writeln!(
                    source,
                    "  # Method {method}.\n  # @param [String] value input\n  def method_{method}(value)\n    value\n  end"
                );
            }
            source.push_str("end\n\n");
        }
        facts.push(parse_source(format!("generated/file_{file}.rb"), &source));
    }

    let started = Instant::now();
    let registry = build_registry(&facts);
    let elapsed = started.elapsed();

    let methods = registry
        .objects()
        .iter()
        .filter(|object| object.kind == meadow_model::ObjectKind::Method)
        .count();
    assert_eq!(methods, 12_000);
    assert!(
        elapsed.as_secs() < 30,
        "handling took too long: {elapsed:?}"
    );
    println!(
        "synthetic corpus handled in {elapsed:?} ({} objects)",
        registry.objects().len()
    );
}
