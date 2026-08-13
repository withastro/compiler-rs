//! Text-escaping helpers for text and attribute payloads emitted as TSX.

/// Escape for text wrapped in `{\`...\`}`: backslashes, backticks, and `${`.
pub(crate) fn template_text_escape(ch: char, next: Option<char>) -> Option<&'static str> {
    match ch {
        '\\' => Some("\\\\"),
        '`' => Some("\\`"),
        '$' if next == Some('{') => Some("\\$"),
        _ => None,
    }
}

/// Escape for `{/** ... */}` bodies, where `*/` closes the comment early.
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

/// Returns the synthetic component identifier emitted as the default export.
///
/// Empty and `<stdin>` filenames produce the bare placeholder; anything else
/// gets the camel-cased basename prefix when it is a valid identifier.
/// Identifier validity is tested ASCII-only.
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
    // A dynamic route (`[slug].astro`) still names its component after the param.
    let camel = to_camel_case(basename.trim_matches(['[', ']', '.']));
    if is_ascii_identifier(&camel) {
        format!("{camel}{PLACEHOLDER}")
    } else {
        PLACEHOLDER.to_string()
    }
}

fn to_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut capitalize_next = true;
    for ch in input.chars() {
        if ch == '-' || ch == '_' || ch == ' ' {
            capitalize_next = true;
            continue;
        }
        if capitalize_next {
            out.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_ascii_identifier(input: &str) -> bool {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
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

/// Validates an attribute name for direct emission into JSX.
///
/// JSX accepts `[a-zA-Z_$][a-zA-Z0-9_$]*` plus `:` (namespace) and `-`
/// (custom attributes). Anything else — `@click`, `:class`, `x-on:k.s.e` —
/// must be wrapped in a spread object literal.
pub(crate) fn is_valid_tsx_attribute_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_valid_first_ident_char(first) {
        return false;
    }
    chars.all(|ch| is_valid_first_ident_char(ch) || ch.is_ascii_digit() || ch == ':' || ch == '-')
}

fn is_valid_first_ident_char(ch: char) -> bool {
    ch == '$' || ch == '_' || ch.is_alphabetic()
}

/// The family a script's `type` attribute puts it in.
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
