use std::borrow::Cow;

use biome_string_case::Case;
use biome_unicode_table::{is_js_id_continue, is_js_id_start, is_js_ident};

pub(crate) fn template_text_escape(ch: char, next: Option<char>) -> Option<&'static str> {
    match ch {
        '\\' => Some("\\\\"),
        '`' => Some("\\`"),
        '$' if next == Some('{') => Some("\\$"),
        _ => None,
    }
}

pub(crate) fn comment_body_escape(previous: Option<char>, ch: char) -> Option<&'static str> {
    match ch {
        '\\' => Some("\\\\"),
        '{' => Some("\\\\{"),
        '}' => Some("\\\\}"),
        '/' if previous == Some('*') => Some("\\/"),
        _ => None,
    }
}

pub(crate) fn comment_needs_leading_space(body: &str) -> bool {
    body.chars().next().is_none_or(|c| !c.is_whitespace())
}

pub(crate) fn escape_javascript_string(src: &str) -> String {
    serde_json::to_string(src)
        .expect("serializing a string cannot fail")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub(crate) fn decode_html_entities(src: &str) -> Cow<'_, str> {
    htmlize::unescape_attribute(src)
}

pub(crate) fn strip_matching_quotes(text: &str) -> Option<&str> {
    let quote = match text.chars().next()? {
        quote @ ('"' | '\'') => quote,
        _ => return None,
    };
    text.strip_prefix(quote)?.strip_suffix(quote)
}

pub(crate) fn tsx_component_names(filename: Option<&str>) -> (String, Option<String>) {
    let fallback = || ("AstroComponent".to_string(), None);
    let Some(filename) = filename.filter(|name| !name.is_empty() && *name != "<stdin>") else {
        return fallback();
    };
    let last_segment = filename.rsplit(['/', '\\']).next().unwrap_or("");
    let basename = last_segment
        .rsplit_once('.')
        .map_or(last_segment, |(name, _)| name);
    if basename == "404" {
        return ("FourOhFourAstroComponent".to_string(), None);
    }
    let stem = last_segment.split('.').next().unwrap_or("");
    let pascal = Case::Pascal.convert(stem);
    if !is_js_ident(&pascal) {
        return fallback();
    }
    if basename.starts_with('[') && basename.ends_with(']') {
        return (format!("_{pascal}_AstroComponent"), None);
    }
    let alias = (pascal == clean_component_name(basename)).then(|| pascal.to_string());
    (format!("{pascal}AstroComponent"), alias)
}

fn clean_component_name(basename: &str) -> String {
    let filtered: String = basename
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '-'))
        .collect();
    let first_letter = filtered.find(|ch: char| ch.is_ascii_alphabetic());
    let trimmed = &filtered[first_letter.unwrap_or(0)..];
    let mut name = String::new();
    if first_letter.is_none() {
        name.push('A');
    }
    let mut words = trimmed.split(['-', '_']).filter(|word| !word.is_empty());
    if let Some(first) = words.next() {
        name.extend(first.chars().filter(|ch| *ch != '$'));
        if let Some(first) = name.get_mut(..1) {
            first.make_ascii_uppercase();
        }
    }
    for word in words {
        let word = word.replace('$', "");
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(chars.map(|ch| ch.to_ascii_lowercase()));
        }
    }
    name
}

pub(crate) fn is_html_event_attribute(name: &str) -> bool {
    matches!(
        name,
        "ontouchcancel"
            | "ontouchend"
            | "ontouchmove"
            | "ontouchstart"
            | "onwebkitanimationend"
            | "onwebkitanimationiteration"
            | "onwebkitanimationstart"
            | "onwebkittransitionend"
            | "ongamepadconnected"
            | "ongamepaddisconnected"
            | "onpagereveal"
            | "onpageswap"
            | "onanimationcancel"
            | "onanimationend"
            | "onanimationiteration"
            | "onanimationstart"
            | "onbeforeinput"
            | "onbeforetoggle"
            | "oncompositionend"
            | "oncompositionstart"
            | "oncompositionupdate"
            | "onfocusin"
            | "onfocusout"
            | "ongotpointercapture"
            | "onlostpointercapture"
            | "onpointercancel"
            | "onpointerdown"
            | "onpointerenter"
            | "onpointerleave"
            | "onpointermove"
            | "onpointerout"
            | "onpointerover"
            | "onpointerrawupdate"
            | "onpointerup"
            | "onselectionchange"
            | "onselectstart"
            | "ontransitioncancel"
            | "ontransitionend"
            | "ontransitionrun"
            | "ontransitionstart"
            | "onabort"
            | "onafterprint"
            | "onauxclick"
            | "onbeforematch"
            | "onbeforeprint"
            | "onbeforeunload"
            | "onblur"
            | "oncancel"
            | "oncanplay"
            | "oncanplaythrough"
            | "onchange"
            | "onclick"
            | "onclose"
            | "oncontextlost"
            | "oncontextmenu"
            | "oncontextrestored"
            | "oncopy"
            | "oncuechange"
            | "oncut"
            | "ondblclick"
            | "ondrag"
            | "ondragend"
            | "ondragenter"
            | "ondragleave"
            | "ondragover"
            | "ondragstart"
            | "ondrop"
            | "ondurationchange"
            | "onemptied"
            | "onended"
            | "onerror"
            | "onfocus"
            | "onformdata"
            | "onhashchange"
            | "oninput"
            | "oninvalid"
            | "onkeydown"
            | "onkeypress"
            | "onkeyup"
            | "onlanguagechange"
            | "onload"
            | "onloadeddata"
            | "onloadedmetadata"
            | "onloadstart"
            | "onmessage"
            | "onmessageerror"
            | "onmousedown"
            | "onmouseenter"
            | "onmouseleave"
            | "onmousemove"
            | "onmouseout"
            | "onmouseover"
            | "onmouseup"
            | "onoffline"
            | "ononline"
            | "onpagehide"
            | "onpageshow"
            | "onpaste"
            | "onpause"
            | "onplay"
            | "onplaying"
            | "onpopstate"
            | "onprogress"
            | "onratechange"
            | "onrejectionhandled"
            | "onreset"
            | "onresize"
            | "onscroll"
            | "onscrollend"
            | "onsecuritypolicyviolation"
            | "onseeked"
            | "onseeking"
            | "onselect"
            | "onslotchange"
            | "onstalled"
            | "onstorage"
            | "onsubmit"
            | "onsuspend"
            | "ontimeupdate"
            | "ontoggle"
            | "onunhandledrejection"
            | "onunload"
            | "onvolumechange"
            | "onwaiting"
            | "onwheel"
    )
}

pub(crate) fn is_valid_tsx_attribute_name(name: &str) -> bool {
    let mut parts = name.split(':');
    let valid_part = |part: &str| {
        let mut chars = part.chars();
        chars.next().is_some_and(is_js_id_start)
            && chars.all(|ch| is_js_id_continue(ch) || ch == '-')
    };
    valid_part(parts.next().unwrap_or_default())
        && parts.next().is_none_or(valid_part)
        && parts.next().is_none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyMode {
    Normal,
    Script,
    Style,
    Raw,
    TextOnly,
    NoExpressions,
}

pub(crate) fn body_mode(name: Option<&str>, is_raw: bool) -> BodyMode {
    let name = name.unwrap_or_default().to_ascii_lowercase();
    match name.as_str() {
        "script" => BodyMode::Script,
        "style" => BodyMode::Style,
        _ if is_raw => BodyMode::Raw,
        "math" => BodyMode::NoExpressions,
        "iframe" | "noembed" | "noframes" | "plaintext" | "xmp" => BodyMode::Raw,
        "title" | "textarea" => BodyMode::TextOnly,
        _ => BodyMode::Normal,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScriptKind {
    Script,
    Json,
    Unknown,
}

pub(crate) fn classify_script_type(type_value: Option<&str>) -> ScriptKind {
    match type_value {
        None => ScriptKind::Script,
        Some(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            if matches!(
                normalized.as_str(),
                "" | "module"
                    | "text/javascript"
                    | "application/ecmascript"
                    | "application/x-ecmascript"
                    | "application/x-javascript"
                    | "text/ecmascript"
                    | "text/javascript1.0"
                    | "text/javascript1.1"
                    | "text/javascript1.2"
                    | "text/javascript1.3"
                    | "text/javascript1.4"
                    | "text/javascript1.5"
                    | "text/jscript"
                    | "text/livescript"
                    | "text/x-ecmascript"
                    | "text/x-javascript"
                    | "text/typescript"
                    | "application/javascript"
                    | "text/partytown"
                    | "application/node"
            ) {
                ScriptKind::Script
            } else if matches!(
                normalized.as_str(),
                "application/json" | "application/ld+json" | "importmap" | "speculationrules"
            ) {
                ScriptKind::Json
            } else {
                ScriptKind::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tsx_component_names;
    use crate::{ConvertOptions, convert_to_tsx};

    #[test]
    fn component_names_match_editor_exports() {
        for (filename, component, alias) in [
            ("MyPage.astro", "MyPageAstroComponent", Some("MyPage")),
            ("image.astro", "ImageAstroComponent", Some("Image")),
            ("my-comp.astro", "MyCompAstroComponent", Some("MyComp")),
            ("my_comp.astro", "MyCompAstroComponent", Some("MyComp")),
            (
                "file:///src/MyPage.astro",
                "MyPageAstroComponent",
                Some("MyPage"),
            ),
            ("[slug].astro", "_Slug_AstroComponent", None),
            ("404.astro", "FourOhFourAstroComponent", None),
            ("[...path].astro", "AstroComponent", None),
            ("123.astro", "AstroComponent", None),
            (".astro", "AstroComponent", None),
            ("My.Page.astro", "MyAstroComponent", None),
            ("Ünicorn.astro", "ÜnicornAstroComponent", None),
            ("", "AstroComponent", None),
            ("<stdin>", "AstroComponent", None),
        ] {
            let (actual, clean) = tsx_component_names(Some(filename));
            assert_eq!(actual, component, "{filename}");
            assert_eq!(clean.as_deref(), alias, "{filename}");
        }
        assert_eq!(tsx_component_names(None), ("AstroComponent".into(), None));
    }

    #[test]
    fn windows_paths_use_the_filename_for_component_names() {
        let result = convert_to_tsx(
            "<p/>",
            ConvertOptions {
                filename: Some(r"C:\repo\src\MyPage.astro".to_string()),
                ..Default::default()
            },
        );
        assert!(result.code.contains("function MyPageAstroComponent("));
        assert!(
            result
                .code
                .contains("export { MyPageAstroComponent as MyPage };")
        );
    }
}
