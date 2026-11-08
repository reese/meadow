//! Dumps the owned event stream for a Ruby file as pretty JSON.
//!
//! Usage: `cargo run -p meadow-parser --example dump_events -- path/to/file.rb`

use std::path::PathBuf;

fn main() {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: dump_events <file.rb>");
        std::process::exit(2);
    };

    let facts = match meadow_parser::parse_file(&path) {
        Ok(facts) => facts,
        Err(error) => {
            eprintln!("failed to read {}: {error}", path.display());
            std::process::exit(1);
        }
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&facts).expect("serialize facts")
    );
}
