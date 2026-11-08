//! Arbitrary input must never crash the parser process.

use std::panic::{AssertUnwindSafe, catch_unwind};

fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed
        .wrapping_mul(2_862_933_555_777_941_757)
        .wrapping_add(3_037_000_493);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state & 0xff) as u8
        })
        .collect()
}

#[test]
fn parses_arbitrary_bytes_without_panicking() {
    for seed in 0..500 {
        let input = bytes(seed, usize::try_from(seed % 256).unwrap_or(0) + 1);
        let result = catch_unwind(AssertUnwindSafe(|| {
            meadow_parser::parse_source_bytes("arbitrary.rb", &input)
        }));
        assert!(result.is_ok(), "parser panicked for {input:?}");
    }
}

use proptest::prelude::*;

proptest! {
    #[test]
    fn proptest_parses_arbitrary_bytes(input in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = meadow_parser::parse_source_bytes("arbitrary.rb", &input);
    }
}
