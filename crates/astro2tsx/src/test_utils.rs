use biome_js_parser::{JsParserOptions, parse};
use biome_languages::JsFileSource;

use crate::{ConvertOptions, ConvertResult, convert_to_tsx};

pub(crate) fn convert(source: &str) -> ConvertResult {
    let result = convert_to_tsx(source, ConvertOptions::default());
    assert!(
        !result.has_parse_errors,
        "{source}: {:?}",
        result.diagnostics
    );
    let parsed = parse(
        &result.code,
        JsFileSource::tsx(),
        JsParserOptions::default(),
    );
    assert!(
        parsed.diagnostics().is_empty(),
        "{}: {:?}",
        result.code,
        parsed.diagnostics()
    );
    assert_mapped_runs_are_verbatim(source, &result, source);
    result
}

pub(crate) fn assert_mapped_runs_are_verbatim(source: &str, result: &ConvertResult, label: &str) {
    let code_len = result.code.len() as u32;
    let mut previous_generated = 0;
    for (index, mapping) in result.mappings.iter().enumerate() {
        assert!(
            mapping.generated >= previous_generated,
            "{label}: run {index} goes backwards"
        );
        previous_generated = mapping.generated;
        let Some(original) = mapping.original else {
            continue;
        };
        let run_end = result
            .mappings
            .get(index + 1)
            .map(|next| next.generated)
            .unwrap_or(code_len);
        let length = (run_end - mapping.generated).min(source.len() as u32 - original);
        let generated_slice =
            &result.code[mapping.generated as usize..(mapping.generated + length) as usize];
        let source_slice = &source[original as usize..(original + length) as usize];
        assert_eq!(
            generated_slice, source_slice,
            "{label}: run {index} is not verbatim"
        );
    }
}
