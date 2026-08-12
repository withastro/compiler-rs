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
    let mut cases = 0usize;
    for src in CONSTRUCTS {
        for end in 0..=src.len() {
            if !src.is_char_boundary(end) {
                continue;
            }
            let t = &src[..end];
            cases += 1;
            let r = convert_to_tsx(
                t,
                ConvertOptions {
                    filename: Some("index.astro".into()),
                },
            );
            assert!(!r.code.is_empty(), "empty output for {t:?}");
            assert!(
                r.code.starts_with("/* @jsxImportSource astro */"),
                "missing prefix for {t:?}"
            );
        }
    }
    eprintln!("truncation cases: {cases}, panics: 0, empty: 0");
}
