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
// Occurrence disambiguates basic.ts's two same-named "preserves spaces in tag" cases.
const INTENTIONAL_DIVERGENCES: &[&str] = &[
    "escape.ts::does not escape tag opening unnecessarily II [1]",
    "escape.ts::does not escape tag opening unnecessarily III [1]",
    "basic.ts::preserves spaces in tag [2]",
    "basic.ts::preserves line returns in tag by transforming to space [1]",
];

fn record_key(record: &Record, occurrence: usize) -> String {
    format!("{}::{} [{occurrence}]", record.file, record.name)
}

#[derive(Debug, Deserialize)]
struct Record {
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
fn corpus_parity() {
    let corpus = load_corpus();
    assert!(!corpus.is_empty(), "corpus is empty — regenerate it");

    let mut occurrences: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut keys: Vec<String> = Vec::new();
    let mut regressions: Vec<String> = Vec::new();
    let mut resolved: Vec<String> = Vec::new();

    for record in &corpus {
        let occurrence = occurrences
            .entry((record.file.clone(), record.name.clone()))
            .or_insert(0);
        *occurrence += 1;
        let key = record_key(record, *occurrence);
        keys.push(key.clone());

        let actual = convert_to_tsx(
            &record.input,
            ConvertOptions {
                filename: record.options.filename.clone(),
            },
        )
        .code;

        let matched = strip_inline_sourcemap(&actual) == strip_inline_sourcemap(&record.expected);
        let allowed = INTENTIONAL_DIVERGENCES.contains(&key.as_str());

        match (matched, allowed) {
            (false, false) => regressions.push(format!(
                "  {key}\n    input:    {:?}\n    expected: {:?}\n    actual:   {:?}",
                record.input,
                strip_inline_sourcemap(&record.expected),
                strip_inline_sourcemap(&actual),
            )),
            (true, true) => resolved.push(key),
            _ => {}
        }
    }

    let unmatched: Vec<&str> = INTENTIONAL_DIVERGENCES
        .iter()
        .copied()
        .filter(|listed| !keys.iter().any(|key| key == listed))
        .collect();

    let mut failure = String::new();
    if !regressions.is_empty() {
        failure.push_str(&format!(
            "{} case(s) diverge but are not listed in INTENTIONAL_DIVERGENCES:\n{}\n",
            regressions.len(),
            regressions.join("\n"),
        ));
    }
    if !resolved.is_empty() {
        failure.push_str(&format!(
            "{} listed divergence(s) now match and must be removed from the list:\n  {}\n",
            resolved.len(),
            resolved.join("\n  "),
        ));
    }
    if !unmatched.is_empty() {
        failure.push_str(&format!(
            "{} listed divergence(s) match no corpus record:\n  {}\n",
            unmatched.len(),
            unmatched.join("\n  "),
        ));
    }
    assert!(failure.is_empty(), "\n{failure}");
}
