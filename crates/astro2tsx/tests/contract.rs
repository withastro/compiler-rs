use astro2tsx::{ConvertOptions, convert_to_tsx};
const CONSTRUCTS: &[&str] = &[
    "<div>{\"<br/>\"}</div>",
    "<div>{'<b>y</b>'}</div>",
    "<div>{`<b>y</b>`}</div>",
    "<div>{foo<Bar>(x)}</div>",
    "<div>{a.b<C<D>>(y)}</div>",
    "<div>{c && <span>a</span> <span>b</span>}</div>",
    "<div>{l.map(i => <span>{i}</span>)}</div>",
    "{list.map(() => <Component><!--Hi--></Component>)}",
    "<div>{x && <span><!--hi--></span>}</div>",
    "<div>{...rest}</div>",
    "<div>{ {a:1} }</div>",
    "<div>{/* c */}</div>",
    "hello\n---\nconst a = 1;\n---\n<p>x</p>",
    "<div a= b=\"c\"></div>",
    "<Comp a='it\\'s' />",
    "<div class=`></div>",
    "---\nconst x = 1;\n---\n<div class=\"a\">{x}<style>.a{color:red}</style></div>",
];
#[test]
fn never_fails_on_truncated_input() {
    for src in CONSTRUCTS {
        for end in 0..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let t = &src[..end];
            let r = convert_to_tsx(
                t,
                ConvertOptions {
                    filename: Some("index.astro".into()),
                    ..Default::default()
                },
            );
            assert!(!r.code.is_empty(), "empty output for {t:?}");
            assert!(
                r.code.starts_with("/* @jsxImportSource astro */"),
                "missing prefix for {t:?}"
            );
        }
    }
}

fn convert(source: &str) -> astro2tsx::ConvertResult {
    convert_to_tsx(source, ConvertOptions::default())
}

#[test]
fn script_text_is_excluded_but_the_tag_remains() {
    let result = convert("<script>const a = 1;</script>");
    assert!(result.code.contains("<script>"));
    assert!(result.code.contains("</script>"));
    assert!(!result.code.contains("const a = 1;"));
}

#[test]
fn json_script_text_is_excluded_but_the_tag_remains() {
    let result = convert("<script type=\"application/json\">{\"a\":1}</script>");
    assert!(result.code.contains("<script type=\"application/json\">"));
    assert!(!result.code.contains("\"a\":1"));
}

#[test]
fn unknown_script_text_is_excluded_but_the_tag_remains() {
    let result = convert("<script type=\"text/nonsense\">wat wat</script>");
    assert!(result.code.contains("<script type=\"text/nonsense\">"));
    assert!(!result.code.contains("wat wat"));
}

#[test]
fn style_text_is_excluded_but_the_tag_remains() {
    let result = convert("<style>.a { color: red; }</style>");
    assert!(result.code.contains("<style>"));
    assert!(result.code.contains("</style>"));
    assert!(!result.code.contains("color: red"));
}

#[test]
fn raw_text_is_still_emitted() {
    let result = convert("<div is:raw><b>not markup</b></div>");
    assert!(
        result.code.contains("{`<b>not markup</b>`}"),
        "is:raw content is not gated:\n{}",
        result.code
    );
}

#[test]
fn script_and_style_win_over_is_raw() {
    let script = convert("<script is:raw>const a = 1;</script>");
    assert!(!script.code.contains("const a = 1;"), "{}", script.code);

    let style = convert("<style is:raw>.a { color: red; }</style>");
    assert!(!style.code.contains("color: red"), "{}", style.code);
}

#[test]
fn excluded_bodies_are_still_reported_as_extracted_tags() {
    let result = convert("<script>const a = 1;</script><style>.a{color:red}</style>");
    assert_eq!(result.scripts.len(), 1);
    assert_eq!(result.scripts[0].content, "const a = 1;");
    assert_eq!(result.styles.len(), 1);
    assert_eq!(result.styles[0].content, ".a{color:red}");
}
