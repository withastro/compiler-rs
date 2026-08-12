//! Behavioral, error-recovery, and sourcemap coverage for TSX conversion.

use astro2tsx::{ConvertOptions, Mapping, convert_to_tsx};

fn convert(input: &str) -> astro2tsx::ConvertResult {
    convert_to_tsx(
        input,
        ConvertOptions {
            filename: Some("index.astro".to_string()),
            ..Default::default()
        },
    )
}

/// Line (1-based) and column (0-based, UTF-16 code units) of a byte offset.
fn line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 0usize;
    for (idx, ch) in source.char_indices() {
        if idx >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16();
        }
    }
    (line, col)
}

fn snippet_offset(source: &str, snippet: &str) -> usize {
    source
        .find(snippet)
        .unwrap_or_else(|| panic!("snippet {snippet:?} not found in input"))
}

/// Greatest mapping whose `original` is at or before `byte`.
fn original_to_generated(mappings: &[Mapping], byte: u32) -> Option<u32> {
    mappings
        .iter()
        .filter(|m| m.original.is_some_and(|o| o <= byte))
        .max_by_key(|m| m.original.unwrap())
        .map(|m| m.generated)
}

enum Resolution {
    /// Mapped offset lands exactly on the snippet in the generated output.
    Exact,
    /// Mapped offset lands elsewhere; carries the generated text found there.
    Wrong(String),
    Unmapped,
}

/// Resolves a snippet's original offset to a generated offset and reports
/// whether the generated output actually carries the snippet there.
fn resolve(input: &str, snippet: &str) -> Resolution {
    let result = convert(input);
    let offset = snippet_offset(input, snippet) as u32;
    let Some(generated) = original_to_generated(&result.mappings, offset) else {
        return Resolution::Unmapped;
    };
    let exact = result
        .mappings
        .iter()
        .any(|m| m.original == Some(offset) && m.generated == generated);
    let at = result.code.get(generated as usize..).unwrap_or("");
    if exact && at.starts_with(snippet) {
        Resolution::Exact
    } else {
        let found: String = at.chars().take(snippet.chars().count().max(12)).collect();
        Resolution::Wrong(found)
    }
}

const SOURCEMAP_CASES: &[(&str, &str, &str)] = &[
    ("attributes/shorthand", "<div {name} />", "name"),
    ("attributes/empty-quoted", "<div src=\"\" />", "\""),
    (
        "attributes/template-literal",
        "---\n---\n<Tag src=`bar${foo}` />",
        "foo",
    ),
    (
        "attributes/multiline-quoted",
        "<path d=\"M 0\nC100 0\nZ\" />",
        "Z",
    ),
    (
        "deprecated/jsdoc",
        "---\n    /** @deprecated */\nconst deprecated = \"Astro\"\ndeprecated;\nconst hello = \"Astro\"\n---\n",
        "deprecated;",
    ),
    (
        "error/svelte",
        "---\nimport SvelteOptionalProps from \"./SvelteOptionalProps.svelte\"\n---\n\n<SvelteOptionalProps></SvelteOptionalProps>",
        "<SvelteOptionalProps>",
    ),
    (
        "error/vue",
        "---\nimport SvelteError from \"./SvelteError.svelte\"\nimport VueError from \"./VueError.vue\"\n---\n\n<SvelteError></SvelteError>\n<VueError></VueError>",
        "<VueError>",
    ),
    ("frontmatter/bare", "---\nnonexistent\n---\n", "nonexistent"),
    (
        "hover/plain",
        "---\n    const MyVariable = \"Astro\"\n\n    /** Documentation */\n    const MyDocumentedVariable = \"Astro\"\n\n    /** @author Astro */\n    const MyJSDocVariable = \"Astro\"\n---\n",
        "MyVariable",
    ),
    (
        "hover/documented",
        "---\n    const MyVariable = \"Astro\"\n\n    /** Documentation */\n    const MyDocumentedVariable = \"Astro\"\n\n    /** @author Astro */\n    const MyJSDocVariable = \"Astro\"\n---\n",
        "MyDocumentedVariable",
    ),
    (
        "hover/jsdoc",
        "---\n    const MyVariable = \"Astro\"\n\n    /** Documentation */\n    const MyDocumentedVariable = \"Astro\"\n\n    /** @author Astro */\n    const MyJSDocVariable = \"Astro\"\n---\n",
        "MyJSDocVariable",
    ),
    (
        "module/invalid-import",
        "---\n  // valid\n  import { foo } from './script.js';\n    import ComponentAstro from './astro.astro';\n  // invalid\n  import { baz } from './script';\n  foo;baz;ComponentAstro;\n---\n",
        "'./script'",
    ),
    ("multibyte/content", "<h1>ツ</h1>", "ツ"),
    ("multibyte/after", "<h1>ツ</h1><p>foobar</p>", "foobar"),
    ("multibyte/many-n", "<h1>こんにちは</h1>", "ん"),
    ("multibyte/many-ni", "<h1>こんにちは</h1>", "に"),
    ("tags/close", "<Hello></Hello>", ">"),
    ("template/basic", "<div>{nonexistent}</div>", "nonexistent"),
    ("template/dot", "<div>{console.log(hey)}</div>", "log"),
    ("template/addition", "{\"hello\" + hey}", "hey"),
    (
        "template/html-attribute",
        "<svg color=\"#000\"></svg>",
        "color",
    ),
    (
        "template/complex-item",
        "{[].map(ITEM => {\nv = \"what\";\nreturn <div>{ITEMS}</div>\n})}",
        "ITEM",
    ),
    (
        "template/complex-items",
        "{[].map(ITEM => {\nv = \"what\";\nreturn <div>{ITEMS}</div>\n})}",
        "ITEMS",
    ),
    (
        "template/attributes",
        "<div className=\"hello\" />",
        "className",
    ),
    (
        "template/special-attributes",
        "<div @on.click=\"fn\" />",
        "@on.click",
    ),
    (
        "windows/crlf-last-char",
        "---\r\nimport { Meta } from '$lib/components/Meta.astro';\r\n---\r\n",
        ";",
    ),
    (
        "windows/template-basic",
        "<div>{\r\nnonexistent\r\n}</div>",
        "nonexistent",
    ),
    (
        "windows/template-dot-lf",
        "<div>{\nconsole.log(hey)\n}</div>",
        "log",
    ),
    (
        "windows/template-dot-crlf",
        "<div>{\r\nconsole.log(hey)\r\n}</div>",
        "log",
    ),
    ("windows/addition-lf", "{\"hello\" + \nhey}", "hey"),
    ("windows/addition-crlf", "{\"hello\" + \r\nhey}", "hey"),
    (
        "windows/html-attribute-lf",
        "<svg\nvalue=\"foo\" color=\"#000\"></svg>",
        "color",
    ),
    (
        "windows/html-attribute-crlf",
        "<svg\r\nvalue=\"foo\" color=\"#000\"></svg>",
        "color",
    ),
    (
        "windows/complex-items",
        "{[].map(ITEM => {\r\nv = \"what\";\r\nreturn <div>{ITEMS}</div>\r\n})}",
        "ITEMS",
    ),
    (
        "windows/attributes",
        "<div\r\na=\"b\" className=\"hello\" />",
        "className",
    ),
    (
        "windows/special-attributes",
        "<div\r\na=\"b\" @on.click=\"fn\" />",
        "@on.click",
    ),
    (
        "windows/whitespace",
        "---\r\nimport A from \"a\";\r\n\timport B from \"b\";\r\n---\r\n",
        "B",
    ),
];

const KNOWN_GAPS: &[&str] = &[];

#[test]
fn sourcemap_resolves_to_snippet() {
    let mut failures = Vec::new();
    let mut fixed = Vec::new();
    for (label, input, snippet) in SOURCEMAP_CASES {
        let (line, col) = line_col(input, snippet_offset(input, snippet));
        let known = KNOWN_GAPS.contains(label);
        match resolve(input, snippet) {
            Resolution::Exact if known => fixed.push(*label),
            Resolution::Exact => {}
            _ if known => {}
            Resolution::Wrong(found) => failures.push(format!(
                "  {label}: {snippet:?} at {line}:{col} maps onto {found:?}"
            )),
            Resolution::Unmapped => failures.push(format!(
                "  {label}: {snippet:?} at {line}:{col} has no mapping"
            )),
        }
    }

    let mut failure = String::new();
    if !failures.is_empty() {
        failure.push_str(&format!(
            "{}/{} sourcemap positions do not resolve to their snippet:\n{}\n",
            failures.len(),
            SOURCEMAP_CASES.len(),
            failures.join("\n"),
        ));
    }
    if !fixed.is_empty() {
        failure.push_str(&format!(
            "{} KNOWN_GAPS entr(y/ies) now resolve and must be removed:\n  {}\n",
            fixed.len(),
            fixed.join("\n  "),
        ));
    }
    assert!(failure.is_empty(), "\n{failure}");
}

#[test]
fn tolerates_line_separators() {
    for input in [
        "\u{2028}",
        "something\u{2029}something",
        "something\u{2028}\u{2029}",
        "\u{2028}\u{2029}\u{2028}",
    ] {
        let result = convert(input);
        assert!(
            result.code.starts_with("/* @jsxImportSource astro */"),
            "input {input:?} produced no usable output"
        );
    }
}

#[test]
fn handles_non_latin_identifiers() {
    let frontmatter = "var π = Math.PI;\nvar ಠ_ಠ = eval;\nvar ლ_ಠ益ಠ_ლ = 42;\nvar λ = function() {};\nvar Ꙭൽↈⴱ = 'huh';\nvar 〱〱 = 2;\nvar Ⅳ = 4;";
    let input = format!("---\n{frontmatter}\n---\n\n<div></div>\n");
    let result = convert(&input);
    assert!(
        result.code.contains(frontmatter),
        "non-latin frontmatter was not round-tripped verbatim:\n{}",
        result.code
    );
}

#[test]
fn handles_complex_generics() {
    let input = "---\nimport type { GetStaticPaths, MDXInstance } from \"$data/shared\";\n\nexport const getStaticPaths: GetStaticPaths = async () => {\n  const articles = await Astro.glob<Article>(\"/content/articles/**/*.mdx\");\n  return articles.map((article) => {\n    return { params: { slug: getSlugFromFile(article.file) } };\n  });\n};\n\nexport interface Props {\n  article: MDXInstance<Article>;\n}\n\nconst { article } = Astro.props;\n---\n\n<ArticleLayout article={article} />";
    let result = convert(input);
    assert!(
        result.code.contains("MDXInstance<Article>"),
        "complex generics were not preserved:\n{}",
        result.code
    );
    assert!(
        !result.has_parse_errors,
        "complex generics should parse cleanly"
    );
}
