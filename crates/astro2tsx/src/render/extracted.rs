use biome_html_syntax::{AnyHtmlAttribute, AnyHtmlAttributeInitializer, HtmlElement};
use biome_rowan::AstNode;

use crate::printer::range_start;
use crate::types::ExtractedScriptType;
use crate::utils::{ScriptKind, classify_script_type, strip_matching_quotes};

use super::attribute::attribute_key;

/// Only a bare `<script>` is processed by Astro; any attribute (`is:inline`,
/// `define:vars`, …) leaves the script inline, sharing one scope with its peers.
pub(super) fn classify_script(attrs: &[AnyHtmlAttribute]) -> ExtractedScriptType {
    if attrs.is_empty() {
        return ExtractedScriptType::ProcessedModule;
    }
    if attrs
        .iter()
        .any(|a| attribute_key(a).as_deref() == Some("is:raw"))
    {
        return ExtractedScriptType::Raw;
    }
    script_type_for_attr(find_attr_value(attrs, "type"))
}

/// `attr` distinguishes a missing `type` from one whose value is dynamic.
pub(crate) fn script_type_for_attr(attr: Option<Option<String>>) -> ExtractedScriptType {
    match attr {
        None => ExtractedScriptType::Inline,
        Some(None) => ExtractedScriptType::Unknown,
        Some(Some(value)) => match classify_script_type(Some(&value)) {
            ScriptKind::Script => {
                if value.trim().eq_ignore_ascii_case("module") {
                    ExtractedScriptType::Module
                } else {
                    ExtractedScriptType::Inline
                }
            }
            ScriptKind::Json => ExtractedScriptType::Json,
            ScriptKind::Unknown => ExtractedScriptType::Unknown,
        },
    }
}

pub(super) fn style_lang_label(attrs: &[AnyHtmlAttribute]) -> String {
    match find_attr_value(attrs, "lang") {
        None => "css".to_string(),
        Some(None) => "unknown".to_string(),
        Some(Some(value)) => value.trim().to_ascii_lowercase(),
    }
}

/// `None`: absent. `Some(None)`: present but not statically knowable.
/// `Some(Some(value))`: string value with any matching quotes stripped.
fn find_attr_value(attrs: &[AnyHtmlAttribute], name: &str) -> Option<Option<String>> {
    for attr in attrs {
        let AnyHtmlAttribute::HtmlAttribute(attr_node) = attr else {
            continue;
        };
        let Ok(attr_name) = attr_node.name() else {
            continue;
        };
        let Ok(token) = attr_name.value_token() else {
            continue;
        };
        if !token.text_trimmed().eq_ignore_ascii_case(name) {
            continue;
        }
        let Some(initializer) = attr_node.initializer() else {
            return Some(None);
        };
        let Ok(value) = initializer.value() else {
            return Some(None);
        };
        if let AnyHtmlAttributeInitializer::HtmlString(s) = value
            && let Ok(value_token) = s.value_token()
        {
            let raw = value_token.text_trimmed().to_string();
            let inner = strip_matching_quotes(&raw).unwrap_or(&raw);
            return Some(Some(inner.to_string()));
        }
        return Some(None);
    }
    None
}

pub(super) fn inner_range(node: &HtmlElement) -> Option<(u32, u32)> {
    let opening = node.opening_element().ok()?;
    let start = u32::from(opening.r_angle_token().ok()?.text_trimmed_range().end());
    let end = match node
        .closing_element()
        .ok()
        .and_then(|c| c.l_angle_token().ok())
    {
        Some(l_angle) => range_start(l_angle.text_trimmed_range()),
        None => u32::from(node.range().end()),
    };
    if start <= end {
        Some((start, end))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::{ConvertOptions, convert_to_tsx};

    #[test]
    fn unclosed_raw_text_element_still_accounts_for_its_content() {
        let raw = convert_to_tsx("<div is:raw>lost\n<p>after</p>", ConvertOptions::default()).code;
        assert!(
            raw.contains("lost") && raw.contains("after"),
            "is:raw content should stay inline:\n{raw}"
        );

        let style = convert_to_tsx(
            "<style>.a{color:red}\n<div>after</div>",
            ConvertOptions::default(),
        );
        assert!(!style.code.contains("color:red"), "{}", style.code);
        assert_eq!(style.styles.len(), 1);
        assert!(style.styles[0].content.starts_with(".a{color:red}"));

        let script = convert_to_tsx(
            "<script type=\"application/json\">{\"a\":1}",
            ConvertOptions::default(),
        );
        assert!(!script.code.contains("\"a\":1"), "{}", script.code);
        assert_eq!(script.scripts.len(), 1);
        assert_eq!(script.scripts[0].content, "{\"a\":1}");
    }

    #[test]
    fn unclosed_style_still_extracts_its_css() {
        let result = convert_to_tsx("<style>.a{color:red}", ConvertOptions::default());
        assert_eq!(result.styles.len(), 1);
        assert!(result.styles[0].content.contains(".a{color:red}"));
    }

    #[test]
    fn extracted_ranges_are_generated_offsets() {
        let styled = convert_to_tsx("<div style=\"color:red\"></div>", ConvertOptions::default());
        let range = styled.styles[0].range;
        assert_eq!(
            &styled.code[range.start as usize..range.end as usize],
            "color:red"
        );

        let scripted = convert_to_tsx("<div onclick=\"go()\"></div>", ConvertOptions::default());
        let range = scripted.scripts[0].range;
        assert_eq!(
            &scripted.code[range.start as usize..range.end as usize],
            "go()"
        );

        let tag = convert_to_tsx("<style>.a{color:red}</style>", ConvertOptions::default());
        let range = tag.styles[0].range;
        assert_eq!(range.start, range.end);
        assert!(tag.code[..range.start as usize].ends_with("<style>"));
        assert!(tag.code[range.end as usize..].starts_with("</style>"));
    }

    #[test]
    fn extracted_tags_carry_source_ranges() {
        let source = "---\nconst x = 1;\n---\n<style>.a{color:red}</style>\n<div onclick=\"go()\" style=\"color:red\"></div>\n<script>run();</script>";
        let result = convert_to_tsx(source, ConvertOptions::default());

        let slice = |range: crate::SourceRange| &source[range.start as usize..range.end as usize];
        assert_eq!(slice(result.styles[0].source), ".a{color:red}");
        assert_eq!(slice(result.styles[1].source), "color:red");
        assert_eq!(slice(result.scripts[0].source), "go()");
        assert_eq!(slice(result.scripts[1].source), "run();");
    }

    #[test]
    fn components_are_never_treated_as_html_script_or_style() {
        for input in [
            "<Script>alert(1)</Script>",
            "<Style>.a{color:red}</Style>",
            "{cond && <Script>alert(1)</Script>}",
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert!(
                result.scripts.is_empty() && result.styles.is_empty(),
                "{input:?}"
            );
            assert!(
                result.code.contains("alert(1)") || result.code.contains("color:red"),
                "component body was dropped for {input:?}:\n{}",
                result.code
            );
        }
    }

    #[test]
    fn extracted_tag_sources_slice_to_their_content() {
        let source = "---\nconst x = 1;\n---\n<style>.a{color:red}</style>\n<div onclick=\"go()\" style=color:red data-x='a\"b'></div>\n<script>run();</script>";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert!(!result.scripts.is_empty() && !result.styles.is_empty());
        for tag in result.scripts.iter().chain(result.styles.iter()) {
            assert_eq!(
                &source[tag.source.start as usize..tag.source.end as usize],
                tag.content,
                "{tag:?} does not slice back to its content"
            );
        }
    }

    #[test]
    fn extracted_attribute_ranges_slice_to_content_with_inner_quotes() {
        let result = convert_to_tsx(
            "<div style='a:\"x\"' onclick='go(\"y\")'></div>",
            ConvertOptions::default(),
        );
        let style = &result.styles[0];
        assert_eq!(
            &result.code[style.range.start as usize..style.range.end as usize],
            style.content
        );
        let script = &result.scripts[0];
        assert_eq!(
            &result.code[script.range.start as usize..script.range.end as usize],
            script.content
        );
    }

    #[test]
    fn jsx_path_extracts_event_and_style_attributes() {
        let source = "{x && <div onclick=\"go()\" style=\"color:red\"></div>}";
        let result = convert_to_tsx(source, ConvertOptions::default());
        assert_eq!(result.scripts.len(), 1, "{}", result.code);
        assert_eq!(result.scripts[0].content, "go()");
        assert_eq!(result.styles.len(), 1, "{}", result.code);
        assert_eq!(result.styles[0].content, "color:red");
        for tag in result.scripts.iter().chain(result.styles.iter()) {
            assert_eq!(
                &result.code[tag.range.start as usize..tag.range.end as usize],
                tag.content,
                "generated range does not slice to content"
            );
            assert_eq!(
                &source[tag.source.start as usize..tag.source.end as usize],
                tag.content,
                "source range does not slice to content"
            );
        }
    }

    #[test]
    fn script_types_reflect_what_is_statically_knowable() {
        use crate::ExtractedScriptType;

        for (input, expected) in [
            (
                "<script>const a = 1;</script>",
                ExtractedScriptType::ProcessedModule,
            ),
            (
                "<script type=\"module\">x</script>",
                ExtractedScriptType::Module,
            ),
            ("<script is:inline>x</script>", ExtractedScriptType::Inline),
            (
                "<script type={mime}>wat</script>",
                ExtractedScriptType::Unknown,
            ),
            (
                "{x && <script type={mime}>wat</script>}",
                ExtractedScriptType::Unknown,
            ),
        ] {
            let result = convert_to_tsx(input, ConvertOptions::default());
            assert_eq!(
                result.scripts.first().and_then(|tag| tag.script_type),
                Some(expected),
                "wrong script type for {input:?}"
            );
        }
    }

    #[test]
    fn style_languages_match_the_static_attribute_value() {
        for (attribute, expected) in [
            ("", "css"),
            ("lang={lang}", "unknown"),
            ("lang", "unknown"),
            ("lang=\"\"", ""),
            ("lang=\" SCSS \"", "scss"),
            ("lang='LeSs'", "less"),
        ] {
            let result = convert_to_tsx(
                &format!("<style {attribute}>x</style>"),
                ConvertOptions::default(),
            );
            assert_eq!(
                result.styles[0].lang.as_deref(),
                Some(expected),
                "wrong style language for {attribute:?}"
            );
        }
    }
}
