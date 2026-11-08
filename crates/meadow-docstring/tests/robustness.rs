//! Randomized no-panic tests for docstring and type parsing.

use std::panic::{AssertUnwindSafe, catch_unwind};

use meadow_docstring::{DocstringOptions, parse_docstring, parse_type_list_bytes};
use meadow_model::{SourceLocation, SourceText};

fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state & 0xff) as u8
        })
        .collect()
}

fn location() -> SourceLocation {
    SourceLocation {
        start: 0,
        end: 0,
        start_line: 1,
        end_line: 1,
    }
}

#[test]
fn type_parser_never_panics() {
    for seed in 0..1000 {
        let input = bytes(seed, usize::try_from(seed % 96).unwrap_or(0) + 1);
        let result = catch_unwind(AssertUnwindSafe(|| parse_type_list_bytes(&input)));
        assert!(result.is_ok(), "type parser panicked for {input:?}");
    }
}

#[test]
fn docstring_parser_never_panics() {
    let options = DocstringOptions {
        warn_unknown_tags: true,
        validate_params: true,
        parameter_names: vec![SourceText::from("arg")],
    };
    for seed in 0..1000 {
        let input = bytes(seed, usize::try_from(seed % 128).unwrap_or(0) + 1);
        let text = SourceText::from_vec(input.clone());
        let result = catch_unwind(AssertUnwindSafe(|| {
            parse_docstring(&text, location(), &options)
        }));
        assert!(result.is_ok(), "docstring parser panicked for {input:?}");
    }
}

use proptest::prelude::*;

proptest! {
    #[test]
    fn proptest_type_parser_never_panics(input in proptest::collection::vec(any::<u8>(), 0..128)) {
        let _ = parse_type_list_bytes(&input);
    }

    #[test]
    fn proptest_docstring_parser_never_panics(input in "\\PC{0,200}") {
        let text = SourceText::from_bytes(input.as_bytes());
        let _ = parse_docstring(&text, location(), &DocstringOptions::default());
    }
}
