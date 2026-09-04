use biome_string_case::Case;
use oxc_syntax::identifier::{is_identifier_part, is_identifier_start};

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

/// Without the space, an empty body would emit `{/**/}`, which never opens.
pub(crate) fn comment_needs_leading_space(body: &str) -> bool {
    body.chars().next().is_none_or(|c| !c.is_whitespace())
}

pub(crate) fn encode_double_quote(src: &str) -> String {
    src.replace('"', "&quot;")
}

pub(crate) fn strip_matching_quotes(text: &str) -> Option<&str> {
    let quote = match text.chars().next()? {
        quote @ ('"' | '\'') => quote,
        _ => return None,
    };
    text.strip_prefix(quote)?.strip_suffix(quote)
}

pub(crate) fn tsx_component_name(filename: Option<&str>) -> String {
    const PLACEHOLDER: &str = "__AstroComponent_";
    let Some(filename) = filename else {
        return PLACEHOLDER.to_string();
    };
    if filename.is_empty() || filename == "<stdin>" {
        return PLACEHOLDER.to_string();
    }
    let last_segment = filename.rsplit('/').next().unwrap_or("");
    let basename = last_segment.split('.').next().unwrap_or("");
    if basename.is_empty() {
        return PLACEHOLDER.to_string();
    }
    let pascal = Case::Pascal.convert(basename);
    if is_identifier(&pascal) {
        format!("{pascal}{PLACEHOLDER}")
    } else {
        PLACEHOLDER.to_string()
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_identifier_start) && chars.all(is_identifier_part)
}

/// HTML attribute names that map to DOM event handlers — the printer treats
/// their values as inline scripts so consumers can lint them.
pub(crate) fn is_html_event_attribute(name: &str) -> bool {
    matches!(
        name,
        "onabort"
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
    let mut chars = name.chars();
    chars.next().is_some_and(is_identifier_start)
        && chars.all(|ch| is_identifier_part(ch) || ch == ':' || ch == '-')
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
                "module"
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
