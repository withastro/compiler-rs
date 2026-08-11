//! Detects the `Props` type binding and the `getStaticPaths` export in Astro
//! frontmatter, shaping the synthetic `export default function` signature.

use biome_js_parser::{JsParserOptions, parse};
use biome_js_syntax::{
    AnyJsRoot, JsLanguage, JsSyntaxKind, TsInterfaceDeclaration, TsTypeAliasDeclaration,
};
use biome_languages::JsFileSource;
use biome_rowan::{AstNode, AstNodeList, SyntaxNode};

type JsNode = SyntaxNode<JsLanguage>;

#[derive(Debug, Default, Clone)]
pub(crate) struct PropsAnalysis {
    /// `true` when a top-level `Props` binding (interface, type alias,
    /// import, or alias-import) is present.
    pub has_props: bool,
    /// The full generic parameter list, e.g. `<T extends Foo>`. Empty
    /// when there are no type parameters.
    pub generics_decl: String,
    /// Bare generic argument names, e.g. `<T>`. Used at the type-application
    /// site `Props<T>`.
    pub generics_args: String,
    /// `true` when the frontmatter exports a top-level `getStaticPaths`
    /// declaration.
    pub has_get_static_paths: bool,
}

pub(crate) fn analyze(source: &str) -> PropsAnalysis {
    if source.is_empty() {
        return PropsAnalysis::default();
    }
    let parse = parse(source, JsFileSource::astro(), JsParserOptions::default());
    let root: AnyJsRoot = parse.tree();

    let mut analysis = PropsAnalysis::default();

    for top in module_items(&root) {
        let kind = top.kind();
        match kind {
            JsSyntaxKind::TS_INTERFACE_DECLARATION => {
                if let Some(decl) = TsInterfaceDeclaration::cast_ref(&top)
                    && let Ok(id) = decl.id()
                    && id.syntax().text_trimmed() == "Props"
                {
                    analysis.has_props = true;
                    fill_generics_textually(&mut analysis, source, &top);
                }
            }
            JsSyntaxKind::TS_TYPE_ALIAS_DECLARATION => {
                if let Some(decl) = TsTypeAliasDeclaration::cast_ref(&top)
                    && let Ok(id) = decl.binding_identifier()
                    && id.syntax().text_trimmed() == "Props"
                {
                    analysis.has_props = true;
                    fill_generics_textually(&mut analysis, source, &top);
                }
            }
            JsSyntaxKind::JS_IMPORT => {
                let text = top.text_trimmed().to_string();
                if import_binds_props(&text) {
                    analysis.has_props = true;
                }
            }
            _ => {}
        }
        if !analysis.has_get_static_paths && declares_get_static_paths(&top) {
            analysis.has_get_static_paths = true;
        }
    }

    // The JS parser rejects generic forms real fixtures use, e.g.
    // `Props<T extends string ? {...}: never>`.
    if !analysis.has_props {
        text_scan_props_binding(source, &mut analysis);
    }
    if !analysis.has_get_static_paths
        && source.contains("getStaticPaths")
        && source.contains("export")
    {
        analysis.has_get_static_paths = true;
    }

    analysis
}

/// Brace-balanced extraction of the `<...>` immediately following a
/// keyword + `Props` pair.
fn fill_generics_textually(analysis: &mut PropsAnalysis, source: &str, top: &JsNode) {
    let trimmed_range = top.text_trimmed_range();
    let start = u32::from(trimmed_range.start()) as usize;
    let end = (u32::from(trimmed_range.end()) as usize).min(source.len());
    if start >= end {
        return;
    }
    let slice = &source[start..end];
    let Some(props_idx) = slice.find("Props") else {
        return;
    };
    let after_props = props_idx + "Props".len();
    let mut iter = slice[after_props..].bytes().enumerate();
    let mut first_after = None;
    for (i, b) in iter.by_ref() {
        if !(b as char).is_whitespace() {
            first_after = Some((i, b));
            break;
        }
    }
    let Some((relative, b'<')) = first_after else {
        return;
    };
    let l_angle_at = after_props + relative;
    let bytes = slice.as_bytes();
    let mut depth = 1;
    let mut i = l_angle_at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return;
    }
    let r_angle_at = i;
    analysis.generics_decl = slice[l_angle_at..=r_angle_at].to_string();
    analysis.generics_args = collect_top_level_arg_names(&slice[l_angle_at + 1..r_angle_at]);
}

/// Splits `T extends X, P extends Y` on top-level commas (depth-0 brackets
/// only) and returns the leading identifier of each entry, formatted as
/// `<T, P>`. Returns an empty string if no parameters are recognised.
fn collect_top_level_arg_names(inside: &str) -> String {
    let mut args: Vec<String> = Vec::new();
    let bytes = inside.as_bytes();
    let mut depth_angle: i32 = 0;
    let mut depth_curly: i32 = 0;
    let mut depth_paren: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut start = 0usize;
    for i in 0..=bytes.len() {
        let at_end = i == bytes.len();
        let b = if at_end { b',' } else { bytes[i] };
        if !at_end {
            match b {
                b'<' => depth_angle += 1,
                b'>' => depth_angle -= 1,
                b'{' => depth_curly += 1,
                b'}' => depth_curly -= 1,
                b'(' => depth_paren += 1,
                b')' => depth_paren -= 1,
                b'[' => depth_square += 1,
                b']' => depth_square -= 1,
                _ => {}
            }
        }
        let outermost =
            depth_angle == 0 && depth_curly == 0 && depth_paren == 0 && depth_square == 0;
        if b == b',' && outermost {
            let segment = &inside[start..i];
            if let Some(name) = leading_identifier(segment) {
                args.push(name.to_string());
            }
            start = i + 1;
        }
    }
    if args.is_empty() {
        String::new()
    } else {
        format!("<{}>", args.join(", "))
    }
}

fn leading_identifier(segment: &str) -> Option<&str> {
    let trimmed = segment.trim();
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    if end == 0 {
        None
    } else {
        Some(&trimmed[..end])
    }
}

/// Last-resort text scan for `interface Props` / `type Props`, used whenever
/// the tree walk found no binding. Records generics if a `<...>` follows.
fn text_scan_props_binding(source: &str, analysis: &mut PropsAnalysis) {
    for keyword in ["interface", "type"] {
        let mut search_from = 0usize;
        while let Some(pos) = source[search_from..].find(keyword) {
            let abs_pos = search_from + pos;
            let after_kw = abs_pos + keyword.len();
            // `interface` / `type` must be a whole token.
            let before = if abs_pos == 0 {
                b' '
            } else {
                source.as_bytes()[abs_pos - 1]
            };
            if is_ident_byte(before) {
                search_from = after_kw;
                continue;
            }
            // Skip whitespace then expect `Props` followed by ` ` / `<` / `{`.
            let bytes = source.as_bytes();
            let mut i = after_kw;
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if i + "Props".len() <= bytes.len()
                && &bytes[i..i + "Props".len()] == b"Props"
                && (i + "Props".len() == bytes.len() || !is_ident_byte(bytes[i + "Props".len()]))
            {
                analysis.has_props = true;
                let mut j = i + "Props".len();
                while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'<' {
                    let mut depth = 1;
                    let l_angle_at = j;
                    let mut k = j + 1;
                    while k < bytes.len() {
                        match bytes[k] {
                            b'<' => depth += 1,
                            b'>' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    if depth == 0 {
                        analysis.generics_decl = source[l_angle_at..=k].to_string();
                        analysis.generics_args =
                            collect_top_level_arg_names(&source[l_angle_at + 1..k]);
                    }
                }
                return;
            }
            search_from = after_kw;
        }
    }
}

fn module_items(root: &AnyJsRoot) -> Vec<JsNode> {
    match root {
        AnyJsRoot::JsModule(module) => module.items().iter().map(|i| i.into_syntax()).collect(),
        AnyJsRoot::JsScript(script) => script
            .statements()
            .iter()
            .map(|s| s.into_syntax())
            .collect(),
        AnyJsRoot::TsDeclarationModule(decl) => {
            decl.items().iter().map(|i| i.into_syntax()).collect()
        }
        _ => Vec::new(),
    }
}

fn import_binds_props(import_text: &str) -> bool {
    let stripped = strip_comments(import_text);
    scan_props_import(&stripped)
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = i.saturating_add(2);
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn scan_props_import(text: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = b"Props";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let before = if i == 0 { b' ' } else { bytes[i - 1] };
            let after = if i + needle.len() < bytes.len() {
                bytes[i + needle.len()]
            } else {
                b' '
            };
            if !is_ident_byte(before) && !is_ident_byte(after) {
                // `{ Props as Other }` binds `Other`, not `Props`.
                let after_pos = i + needle.len();
                if !next_token_is_as(bytes, after_pos) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

fn next_token_is_as(bytes: &[u8], from: usize) -> bool {
    let mut j = from;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    if j + 2 > bytes.len() {
        return false;
    }
    if &bytes[j..j + 2] == b"as" {
        // Ensure it's the keyword `as`, not an identifier prefix.
        let after = if j + 2 < bytes.len() {
            bytes[j + 2]
        } else {
            b' '
        };
        return !is_ident_byte(after);
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn declares_get_static_paths(node: &JsNode) -> bool {
    let text = node.text_trimmed().to_string();
    if !text.contains("getStaticPaths") {
        return false;
    }
    text.contains("export")
}
