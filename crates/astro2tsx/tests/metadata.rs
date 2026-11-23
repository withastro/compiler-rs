use astro2tsx::{ConvertOptions, FrontmatterStatus, convert_to_tsx};

#[test]
fn generated_section_ranges_exclude_fences_and_fragment_wrappers() {
    for (source, frontmatter_code, body, status) in [
        (
            "---\nconsole.log(\"Hello!\")\n---\n\n<div></div>",
            "console.log(\"Hello!\")\n\n",
            "\n\n<div></div>\n",
            FrontmatterStatus::Closed,
        ),
        (
            "<div></div>",
            "",
            "<div></div>\n",
            FrontmatterStatus::DoesntExist,
        ),
        (
            "---\n\u{1f984}\n---\n\n<div></div>",
            "\u{1f984}\n\n",
            "\n\n<div></div>\n",
            FrontmatterStatus::Closed,
        ),
    ] {
        let result = convert_to_tsx(source, ConvertOptions::default());
        let range = result.frontmatter_range;
        let frontmatter = &result.code[range.start as usize..range.end as usize];
        // The render scope opens between the leading line and the code, so only the ends are fixed.
        if frontmatter_code.is_empty() {
            assert_eq!(frontmatter, "");
        } else {
            assert!(frontmatter.starts_with('\n'), "{frontmatter:?}");
            assert!(frontmatter.ends_with(frontmatter_code), "{frontmatter:?}");
            assert!(!frontmatter.contains("---"), "{frontmatter:?}");
        }
        let range = result.body;
        assert_eq!(&result.code[range.start as usize..range.end as usize], body);
        assert!(result.code[..range.start as usize].ends_with("<Fragment>\n"));
        assert!(result.code[range.end as usize..].starts_with("</Fragment>"));
        assert_eq!(result.frontmatter.status, status);
        assert!(result.scripts.is_empty());
        assert!(result.styles.is_empty());
    }
}
