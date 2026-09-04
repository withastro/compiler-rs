use astro2tsx::{ConvertOptions, SourceMapMode, convert_to_tsx};
use divan::counter::BytesCount;

fn main() {
    divan::main();
}

fn options() -> ConvertOptions {
    ConvertOptions {
        filename: Some("Component.astro".to_string()),
        sourcemap: SourceMapMode::External,
        ..Default::default()
    }
}

fn bench_convert(bencher: divan::Bencher<'_, '_>, source: &str) {
    bencher
        .counter(BytesCount::of_str(source))
        .with_inputs(options)
        .bench_local_values(|options| convert_to_tsx(divan::black_box(source), options));
}

mod components {
    use super::*;

    #[divan::bench]
    fn favicon(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/Favicon.astro"));
    }

    #[divan::bench]
    fn pill_link(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/PillLink.astro"));
    }

    #[divan::bench]
    fn social_links(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/SocialLinks.astro"));
    }

    #[divan::bench]
    fn header_drop_down(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/HeaderDropDown.astro"));
    }

    #[divan::bench]
    fn seo(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/SEO.astro"));
    }

    #[divan::bench]
    fn expression_heavy(bencher: divan::Bencher<'_, '_>) {
        bench_convert(bencher, include_str!("fixtures/ExpressionHeavy.astro"));
    }
}

fn build_page(sections: usize) -> String {
    let fixture = include_str!("fixtures/ExpressionHeavy.astro");
    let (frontmatter, body) = fixture
        .rsplit_once("---\n")
        .expect("fixture has a frontmatter fence");
    let mut source = String::with_capacity(frontmatter.len() + 4 + body.len() * sections);
    source.push_str(frontmatter);
    source.push_str("---\n");
    for _ in 0..sections {
        source.push_str(body);
    }
    source
}

#[divan::bench(args = [8, 64, 256])]
fn large_page(bencher: divan::Bencher<'_, '_>, sections: usize) {
    let source = build_page(sections);
    bencher
        .counter(BytesCount::of_str(&source))
        .with_inputs(options)
        .bench_local_values(|options| convert_to_tsx(divan::black_box(&source), options));
}

mod phases {
    use super::*;

    #[divan::bench]
    fn convert_only(bencher: divan::Bencher<'_, '_>) {
        let source = build_page(64);
        bencher
            .counter(BytesCount::of_str(&source))
            .with_inputs(options)
            .bench_local_values(|options| convert_to_tsx(divan::black_box(&source), options));
    }

    #[divan::bench]
    fn sourcemap_encode(bencher: divan::Bencher<'_, '_>) {
        let source = build_page(64);
        let result = convert_to_tsx(&source, options());
        bencher
            .counter(BytesCount::of_str(&result.code))
            .bench_local(|| result.source_map(divan::black_box(&source), "Component.astro"));
    }

    // Inline maps are the NAPI default exercised in production.
    #[divan::bench]
    fn convert_with_inline_map(bencher: divan::Bencher<'_, '_>) {
        let source = build_page(64);
        bencher
            .counter(BytesCount::of_str(&source))
            .with_inputs(|| ConvertOptions {
                sourcemap: SourceMapMode::Inline,
                ..options()
            })
            .bench_local_values(|options| convert_to_tsx(divan::black_box(&source), options));
    }
}
