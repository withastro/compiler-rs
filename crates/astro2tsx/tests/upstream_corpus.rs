//! Runs every test case in `tests/corpus/tsx.jsonl` (extracted from the
//! upstream Astro compiler) through `convert_to_tsx` and reports parity
//! with the Go reference output.
//!
//! The corpus is regenerated from
//! `packages/compiler/test/extract-corpus.mjs` in the upstream repo. Each
//! entry is `{ file, name, input, options, expected, static_expected }`.

use astro2tsx::{ConvertOptions, convert_to_tsx};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Test cases where we deliberately diverge from the Go printer because
/// its recovery synthesises constructs that don't help intellisense:
///
/// - `<div>\n<div\n</div>` → upstream emits `<div div {...{"<":true}}>`
///   which fabricates phantom `<` and `div` attributes. We round-trip
///   the source instead.
/// - `<Button      >` (unclosed open) → upstream emits a synthetic
///   `</Button>` to balance JSX. tsserver parses unbalanced JSX just
///   fine, so we leave it unbalanced and avoid the synthetic span.
const INTENTIONAL_DIVERGENCES: &[&str] = &[
    "does not escape tag opening unnecessarily II",
    "does not escape tag opening unnecessarily III",
    "preserves spaces in tag",
    "preserves line returns in tag by transforming to space",
];

#[derive(Debug, Deserialize)]
struct Record {
    #[allow(dead_code)]
    file: String,
    name: String,
    input: String,
    #[serde(default)]
    options: Options,
    expected: String,
}

#[derive(Debug, Default, Deserialize)]
struct Options {
    #[serde(default)]
    filename: Option<String>,
}

/// Drops the inline sourcemap comment that the upstream printer appends
/// when `sourcemap: 'inline'`. Our port doesn't yet emit sourcemaps, and
/// the assertion in the underlying test files (`assert.ok(code.includes(
/// ...))`) doesn't depend on the sourcemap.
fn strip_inline_sourcemap(code: &str) -> &str {
    if let Some(idx) = code.rfind("\n//# sourceMappingURL=") {
        &code[..idx]
    } else {
        code
    }
}

fn load_corpus() -> Vec<Record> {
    let raw = include_str!("corpus/tsx.jsonl");
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("malformed corpus entry"))
        .collect()
}

#[test]
fn dump_named_diff() {
    let only = std::env::var("ASTRO_DUMP_NAME").unwrap_or_default();
    if only.is_empty() {
        return;
    }
    let corpus = load_corpus();
    for record in &corpus {
        if !record.name.contains(&only) {
            continue;
        }
        let actual = convert_to_tsx(
            &record.input,
            ConvertOptions {
                filename: record.options.filename.clone(),
            },
        )
        .code;
        eprintln!("\n==== {} :: {} ====", record.file, record.name);
        eprintln!("--- INPUT ---");
        eprintln!("{:?}", record.input);
        eprintln!("--- EXPECTED ---");
        eprintln!("{:?}", strip_inline_sourcemap(&record.expected));
        eprintln!("--- ACTUAL ---");
        eprintln!("{:?}", strip_inline_sourcemap(&actual));
    }
}

#[test]
fn dump_first_diff_per_file() {
    let corpus = load_corpus();
    let mut seen_files: BTreeMap<String, ()> = BTreeMap::new();
    for record in &corpus {
        let actual = convert_to_tsx(
            &record.input,
            ConvertOptions {
                filename: record.options.filename.clone(),
            },
        )
        .code;
        if actual == record.expected {
            continue;
        }
        if seen_files.contains_key(&record.file) {
            continue;
        }
        seen_files.insert(record.file.clone(), ());
        eprintln!("\n==== {} :: {} ====", record.file, record.name);
        eprintln!("--- INPUT ---");
        eprintln!("{:?}", record.input);
        eprintln!("--- EXPECTED ---");
        eprintln!("{:?}", record.expected);
        eprintln!("--- ACTUAL ---");
        eprintln!("{:?}", actual);
    }
}

#[test]
fn report_parity_with_upstream() {
    let corpus = load_corpus();
    assert!(!corpus.is_empty(), "corpus is empty — regenerate it");

    let mut by_file: BTreeMap<String, (usize, usize, Vec<String>, Vec<String>)> =
        BTreeMap::new();

    for record in &corpus {
        let actual = convert_to_tsx(
            &record.input,
            ConvertOptions {
                filename: record.options.filename.clone(),
            },
        )
        .code;

        let entry = by_file
            .entry(record.file.clone())
            .or_insert((0, 0, Vec::new(), Vec::new()));
        entry.1 += 1;
        let expected_trimmed = strip_inline_sourcemap(&record.expected);
        let actual_trimmed = strip_inline_sourcemap(&actual);
        if actual_trimmed == expected_trimmed {
            entry.0 += 1;
        } else if INTENTIONAL_DIVERGENCES.contains(&record.name.as_str()) {
            entry.3.push(record.name.clone());
        } else {
            entry.2.push(record.name.clone());
        }
    }

    let mut total_pass = 0;
    let mut total_diverge = 0;
    let mut total_count = 0;
    eprintln!("\n== upstream tsx test parity ==");
    for (file, (pass, total, failing, divergent)) in &by_file {
        total_pass += pass;
        total_diverge += divergent.len();
        total_count += total;
        eprintln!("  {file}: {pass}/{total}");
        for name in failing {
            eprintln!("    - FAIL: {name}");
        }
        for name in divergent {
            eprintln!("    - DIVERGE (intentional): {name}");
        }
    }
    eprintln!(
        "---\n  total: {total_pass}/{total_count} (+ {total_diverge} intentional divergences)\n"
    );
}
