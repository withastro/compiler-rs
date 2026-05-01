//! Text-escaping helpers, ported from `internal/printer/utils.go` in the
//! upstream Astro compiler. These reproduce the exact transformations applied
//! to text and attribute payloads when emitting TSX.

/// Escapes embedded backslashes, then `${`, then backticks. Used inside
/// `<script>` / `<style>` text-node bodies that are wrapped in
/// `{\`...\`}` so they parse as a TSX template literal.
pub(crate) fn escape_text(src: &str) -> String {
    escape_backticks(&escape_interpolation(&escape_existing_escapes(src)))
}

/// Escapes braces and `${` so a comment body can be embedded in a JSX
/// `{/* ... */}` comment without being interpreted as an expression. The
/// upstream printer doubles each backslash on entry, hence the
/// `escape_existing_escapes` step.
pub(crate) fn escape_braces(src: &str) -> String {
    escape_star_slash(&escape_tsx_expressions(&escape_existing_escapes(src)))
}

fn escape_star_slash(src: &str) -> String {
    src.replace("*/", "*\\/")
}

fn escape_existing_escapes(src: &str) -> String {
    src.replace('\\', "\\\\")
}

fn escape_tsx_expressions(src: &str) -> String {
    src.replace('{', "\\\\{").replace('}', "\\\\}")
}

fn escape_interpolation(src: &str) -> String {
    src.replace("${", "\\${")
}

fn escape_backticks(src: &str) -> String {
    src.replace('`', "\\`")
}

/// Encodes literal `"` as `&quot;` so the value is safe to embed inside a
/// double-quoted JSX attribute.
pub(crate) fn encode_double_quote(src: &str) -> String {
    src.replace('"', "&quot;")
}

/// Returns the synthetic component identifier emitted as the default export.
///
/// Mirrors Go's `getTSXComponentName`: empty / `<stdin>` filenames produce
/// the bare placeholder, and anything else gets the camel-cased basename
/// prefix when it is a valid identifier. We approximate the upstream
/// `IsIdentifier` check with an ASCII-only test, which is enough for every
/// fixture we mirror.
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
    let camel = to_camel_case(basename);
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
/// their values as inline scripts so consumers can lint them. Mirrors the
/// upstream `htmlEvents` table.
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

/// HTML void elements that never have a closing tag. Used to print `/>`
/// instead of an explicit `</tag>` when the source omitted children.
pub(crate) fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Validates an attribute name for direct emission into JSX.
///
/// JSX accepts `[a-zA-Z_$][a-zA-Z0-9_$]*` plus `:` (namespace) and `-`
/// (custom attributes). Anything else — `@click`, `:class`, `x-on:k.s.e` —
/// must be wrapped in a spread object literal. Mirrors
/// `isValidTSXAttribute` from the Go printer.
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

/// Detects the script `type` attribute family. The upstream printer wraps
/// inline / json / unknown payloads in `{\`...\`}` so they parse as TSX
/// template strings, while real module / classic scripts are emitted as
/// `{() => { ... }}` arrow bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScriptKind {
    /// `type="module"` or no `type` and no recognized override — printed as
    /// `{() => { ... }}` so the body is type-checked as JS.
    Script,
    /// JSON-like type — printed as `{\`...\`}`.
    Json,
    /// Any other type attribute — also printed as `{\`...\`}`.
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
