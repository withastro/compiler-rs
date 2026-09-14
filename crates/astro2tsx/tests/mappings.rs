use astro2tsx::{ConvertOptions, convert_to_tsx};

fn assert_mapped(source: &str, snippets: &[&str]) {
    let result = convert_to_tsx(source, ConvertOptions::default());
    assert!(
        !result.has_parse_errors,
        "{source:?}: {:?}",
        result.diagnostics
    );
    for snippet in snippets {
        let start = source.find(snippet).unwrap();
        for (offset, ch) in snippet.char_indices() {
            let original = (start + offset) as u32;
            let generated = result
                .mappings
                .iter()
                .enumerate()
                .find_map(|(index, mapping)| {
                    let delta = original.checked_sub(mapping.original?)?;
                    let end = result
                        .mappings
                        .get(index + 1)
                        .map_or(result.code.len() as u32, |next| next.generated);
                    (delta + ch.len_utf8() as u32 <= end - mapping.generated)
                        .then_some(mapping.generated + delta)
                })
                .unwrap_or_else(|| panic!("{snippet:?} at {original} is unmapped in {source:?}"));
            assert_eq!(
                &result.code[generated as usize..generated as usize + ch.len_utf8()],
                ch.to_string(),
                "{snippet:?} at {original} maps to the wrong text in {source:?}",
            );
            let mapping = result
                .mappings
                .iter()
                .rev()
                .find(|mapping| mapping.generated <= generated)
                .unwrap();
            assert_eq!(
                mapping
                    .original
                    .map(|start| start + generated - mapping.generated),
                Some(original)
            );
        }
    }
}

#[test]
fn attribute_names_and_values_map_back_to_source() {
    for (source, snippets) in [
        ("<div {name} />", vec!["name"]),
        ("<div src=\"\" />", vec!["\"\""]),
        ("---\n---\n<Tag src=`bar${foo}` />", vec!["foo"]),
        ("<path d=\"M 0\nC100 0\nZ\" />", vec!["M 0", "C100 0", "Z"]),
        ("<div @on.click=\"fn\" />", vec!["@on.click"]),
        ("<Hello></Hello>", vec![">", "</Hello>"]),
        ("<Button      ></Button>", vec![">", "</Button>"]),
    ] {
        assert_mapped(source, &snippets);
    }
}

#[test]
fn frontmatter_symbols_and_imports_map_back_to_source() {
    assert_mapped("---\nnonexistent\n---\n", &["nonexistent"]);
    assert_mapped(
        "---\n    /** @deprecated */\nconst deprecated = \"Astro\"\ndeprecated;\nconst hello = \"Astro\"\n---\n",
        &["deprecated;", "hello"],
    );
    assert_mapped(
        "---\n    const MyVariable = \"Astro\"\n\n    /** Documentation */\n    const MyDocumentedVariable = \"Astro\"\n\n    /** @author Astro */\n    const MyJSDocVariable = \"Astro\"\n---\n",
        &["MyVariable", "MyDocumentedVariable", "MyJSDocVariable"],
    );
    assert_mapped(
        "---\n  import { foo } from './script.js';\n    import ComponentAstro from './astro.astro';\n    import ComponentSvelte from './svelte.svelte';\n    import ComponentVue from './vue.vue';\n  import { baz } from './script';\n  foo;baz;ComponentAstro;ComponentSvelte;ComponentVue;\n---\n",
        &[
            "'./script'",
            "'./astro.astro'",
            "'./svelte.svelte'",
            "'./vue.vue'",
        ],
    );
}

#[test]
fn imported_component_tags_map_back_to_source() {
    assert_mapped(
        "---\nimport SvelteOptionalProps from './SvelteOptionalProps.svelte'\nimport SvelteError from './SvelteError.svelte'\nimport VueError from './VueError.vue'\n---\n\n<SvelteOptionalProps></SvelteOptionalProps>\n<SvelteError></SvelteError>\n<VueError></VueError>",
        &["<SvelteOptionalProps>", "<SvelteError>", "<VueError>"],
    );
}

#[test]
fn template_mappings_preserve_lf_and_crlf_positions() {
    for newline in ["\n", "\r\n"] {
        for (source, snippets) in [
            ("<div>{nonexistent}</div>", vec!["nonexistent"]),
            ("<div>{console.log(hey)}</div>", vec!["log", "hey"]),
            ("{\"hello\" + hey}", vec!["hey"]),
            ("<svg color=\"#000\"></svg>", vec!["color"]),
            ("<div className=\"hello\" />", vec!["className"]),
            ("<div>{\nnonexistent\n}</div>", vec!["nonexistent"]),
            ("<div>{\nconsole.log(hey)\n}</div>", vec!["log"]),
            ("{\"hello\" + \nhey}", vec!["hey"]),
            ("<svg\nvalue=\"foo\" color=\"#000\"></svg>", vec!["color"]),
            (
                "{[].map(ITEM => {\nv = \"what\";\nreturn <div>{ITEMS}</div>\n})}",
                vec!["ITEM", "ITEMS"],
            ),
            ("<div\na=\"b\" className=\"hello\" />", vec!["className"]),
            ("<div\na=\"b\" @on.click=\"fn\" />", vec!["@on.click"]),
            (
                "---\nimport { Meta } from '$lib/components/Meta.astro';\n---\n",
                vec![";"],
            ),
            (
                "---\nimport A from \"a\";\n\timport B from \"b\";\n---\n",
                vec!["\timport B"],
            ),
        ] {
            assert_mapped(&source.replace('\n', newline), &snippets);
        }
    }
}

#[test]
fn multibyte_text_and_following_content_map_back_to_source() {
    assert_mapped("<h1>ツ</h1><p>foobar</p>", &["ツ", "foobar"]);
    assert_mapped("<h1>こんにちは</h1>", &["ん", "に"]);
}
