mod attribute;
mod element;
mod extracted;
mod frontmatter;
mod text;

use element::render_element;
pub(crate) use extracted::{script_type_for_attr, style_lang_for_attr};
use text::{comment_trivia_ranges, emit_source_gap};

use biome_html_syntax::HtmlRoot;
use biome_rowan::{AstNode, AstNodeList};

use crate::ConvertOptions;
use crate::printer::{Printer, range_start};
use crate::types::GeneratedRange;
use crate::utils::tsx_component_names;

const TSX_PREFIX: &str = "/* @jsxImportSource astro */\n";

pub(crate) fn render_root(
    printer: &mut Printer,
    root: HtmlRoot,
    options: &ConvertOptions,
) -> (GeneratedRange, Option<GeneratedRange>) {
    printer.comment_ranges = comment_trivia_ranges(&root);
    let frontmatter_node = root.frontmatter();
    printer.map_nil();
    printer.write(TSX_PREFIX);
    let insertion_start = printer.position();
    printer.write("\n");
    if frontmatter_node.is_none() {
        // This generated-only newline is where TypeScript inserts imports. Keep it
        // distinct from the synthetic Fragment so consumers can map edits, but not
        // ordinary source or diagnostics, to the start of the Astro document.
        printer.frontmatter_insertion_range =
            Some(GeneratedRange::new(insertion_start, printer.position()));
    }
    let (component_name, alias) = tsx_component_names(options.filename.as_deref());
    let frontmatter = frontmatter::render(printer, frontmatter_node.as_ref(), alias.as_deref());
    let body = root.html();
    let body_text_start = frontmatter.body_text_start;
    // A childless body still needs its `<Fragment>` when comment trivia remains.
    let has_body_children = body.iter().next().is_some()
        || printer
            .comment_ranges
            .iter()
            .any(|range| range_start(*range) >= body_text_start);
    let body_start;

    if has_body_children {
        // Without a statement boundary, the following JSX can parse as a comparison.
        if frontmatter.needs_terminator {
            printer.map_nil();
            printer.write(";{};");
        }
        printer.map_nil();
        printer.write("<Fragment>\n");
        body_start = printer.position();

        let mut prev_end = body_text_start;
        // TSX has no doctype syntax, so the directive must not reach the output.
        // Its surrounding source gaps remain mapped, including their line breaks.
        if let Some(directive) = root.directive() {
            let directive_range = directive.range();
            emit_source_gap(printer, prev_end, range_start(directive_range));
            prev_end = prev_end.max(u32::from(directive_range.end()));
        }
        for element in body.iter() {
            let element_range = element.range();
            emit_source_gap(printer, prev_end, range_start(element_range));
            render_element(printer, element);
            prev_end = u32::from(element_range.end());
        }
        emit_source_gap(printer, prev_end, printer.source.len() as u32);
        // Keep the closing wrapper on its own line. A trailing source newline
        // therefore produces an intentional blank line before `</Fragment>`.
        printer.map_to_offset(printer.source.len() as u32);
        printer.write("\n");

        let body_end = printer.position();
        printer.body_range = GeneratedRange::new(body_start, body_end);

        printer.map_nil();
        printer.write("</Fragment>\n");
    } else {
        if frontmatter_node.is_some() {
            printer.map_nil();
            printer.write("\n");
        }
        printer.body_range = GeneratedRange::new(printer.position(), printer.position());
    }

    let component_name_range = frontmatter::emit_default_export(
        printer,
        &component_name,
        &frontmatter.props_analysis,
        options.ambient_types,
    );
    let generated_component_export =
        alias
            .filter(|_| !frontmatter.has_component_export)
            .map(|alias| {
                printer.map_nil();
                printer.write("\n");
                let start = printer.position();
                printer.write(&format!("export {{ {component_name} as {alias} }};\n"));
                GeneratedRange::new(start, printer.position())
            });
    (component_name_range, generated_component_export)
}
