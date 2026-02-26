//! Sourcemap builder and post-processing pipeline for Astro codegen.
//!
//! Maps generated JavaScript positions back to original `.astro` source positions.
//! This covers both Phase 1 (Astro codegen) and the Phase 1 + Phase 2 composition
//! pass that runs after TypeScript stripping.
//!
//! The design is adapted from `oxc_codegen`'s `SourcemapBuilder`, but simplified
//! for the Astro codegen's manual printing approach (as opposed to the `Gen` trait).

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::CodegenOptions;
use oxc_span::Span;
use rustc_hash::{FxHashMap, FxHashSet};

/// Sourcemap builder for the Astro codegen pass.
///
/// Tracks the mapping between positions in the generated JavaScript output
/// and positions in the original `.astro` source file.
///
/// The lifetime `'a` ties this builder to the original source text, avoiding
/// the need for `unsafe` transmutes or `'static` references.
pub struct AstroSourcemapBuilder<'a> {
    /// The source id assigned by the inner sourcemap builder.
    source_id: u32,
    /// The original `.astro` source text (used for byte-offset → line/column conversion).
    original_source: &'a str,
    /// Line offset table for the original source: `line_starts[i]` is the byte offset
    /// of the first character on line `i` (0-indexed).
    line_starts: Vec<u32>,
    /// The inner `oxc_sourcemap::SourceMapBuilder` that accumulates tokens.
    inner: oxc_sourcemap::SourceMapBuilder,

    /// Last byte length of the output buffer when we updated generated position.
    last_generated_update: usize,
    /// Current generated line (0-indexed).
    generated_line: u32,
    /// Current generated column (0-indexed, in UTF-16 code units for spec compliance).
    generated_column: u32,
    /// Last original position we emitted a mapping for (used to deduplicate).
    last_position: Option<u32>,
}

impl<'a> AstroSourcemapBuilder<'a> {
    /// Create a new sourcemap builder.
    ///
    /// `source_path` is the filename used in the sourcemap's `sources` array.
    /// `source_text` is the original `.astro` source text, which must outlive
    /// this builder.
    pub fn new(source_path: &Path, source_text: &'a str) -> Self {
        let mut inner = oxc_sourcemap::SourceMapBuilder::default();
        let source_id =
            inner.set_source_and_content(source_path.to_string_lossy().as_ref(), source_text);
        let line_starts = Self::compute_line_starts(source_text);
        Self {
            source_id,
            original_source: source_text,
            line_starts,
            inner,
            last_generated_update: 0,
            generated_line: 0,
            generated_column: 0,
            last_position: None,
        }
    }

    /// Consume the builder and produce the final `SourceMap`.
    pub fn into_sourcemap(self) -> oxc_sourcemap::SourceMap {
        self.inner.into_sourcemap()
    }

    /// Add a source mapping from the current generated position to the given
    /// original byte offset in the `.astro` source.
    ///
    /// `output` is the current contents of the generated code buffer (as bytes).
    /// `original_position` is a byte offset into the original `.astro` source text.
    pub fn add_source_mapping(&mut self, output: &[u8], original_position: u32) {
        self.add_source_mapping_impl(output, original_position, None);
    }

    /// Add a source mapping, bypassing the consecutive-position dedup check.
    ///
    /// Use this when multiple generated lines should map to the same original
    /// position (e.g. a multi-line expression that was expanded by codegen but
    /// originates from a single source span).
    pub fn add_source_mapping_force(&mut self, output: &[u8], original_position: u32) {
        // Temporarily clear last_position so the dedup check passes.
        self.last_position = None;
        self.add_source_mapping_impl(output, original_position, None);
    }

    fn add_source_mapping_impl(
        &mut self,
        output: &[u8],
        original_position: u32,
        name: Option<&str>,
    ) {
        // Deduplicate consecutive mappings to the same original position
        if self.last_position == Some(original_position) {
            return;
        }

        // Clamp position to source length
        let original_position =
            original_position.min(self.original_source.len().try_into().unwrap_or(u32::MAX));

        let (original_line, original_column) = self.byte_offset_to_line_column(original_position);
        self.update_generated_line_and_column(output);

        let name_id = name.map(|n| self.inner.add_name(n));

        self.inner.add_token(
            self.generated_line,
            self.generated_column,
            original_line,
            original_column,
            Some(self.source_id),
            name_id,
        );

        self.last_position = Some(original_position);
    }

    /// Add a mapping for a `Span`, using the span's start position.
    pub fn add_source_mapping_for_span(&mut self, output: &[u8], span: Span) {
        if !span.is_empty() {
            self.add_source_mapping(output, span.start);
        }
    }

    /// Convert a byte offset in the original source to (line, column), both 0-indexed.
    /// Column is counted in UTF-16 code units (per the sourcemap spec).
    #[expect(clippy::cast_possible_truncation)]
    fn byte_offset_to_line_column(&self, byte_offset: u32) -> (u32, u32) {
        // Binary search for the line
        let line = match self.line_starts.binary_search(&byte_offset) {
            Ok(exact) => exact,
            Err(insert_pos) => insert_pos.saturating_sub(1),
        };

        let line_start = self.line_starts[line];
        let byte_col = byte_offset - line_start;

        // Check if the segment is pure ASCII for fast path
        let line_end = byte_offset as usize;
        let line_start_usize = line_start as usize;
        let segment = &self.original_source.as_bytes()
            [line_start_usize..line_end.min(self.original_source.len())];

        let column = if segment.iter().all(u8::is_ascii) {
            byte_col
        } else {
            // Slow path: count UTF-16 code units
            let segment_str =
                &self.original_source[line_start_usize..line_end.min(self.original_source.len())];
            segment_str.encode_utf16().count() as u32
        };

        (line as u32, column)
    }

    /// Update `generated_line` and `generated_column` by scanning new bytes in `output`
    /// since the last update.
    #[expect(clippy::cast_possible_truncation)]
    fn update_generated_line_and_column(&mut self, output: &[u8]) {
        let start = self.last_generated_update;
        if start >= output.len() {
            self.last_generated_update = output.len();
            return;
        }

        let new_bytes = &output[start..];

        // Find the last newline in the new bytes
        let mut last_newline_pos = None;
        let mut newline_count: u32 = 0;

        let mut i = 0;
        while i < new_bytes.len() {
            let b = new_bytes[i];
            if b == b'\n' {
                newline_count += 1;
                last_newline_pos = Some(i);
            } else if b == b'\r' {
                newline_count += 1;
                // Handle \r\n as a single newline; advance past the \n
                if new_bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                    last_newline_pos = Some(i);
                } else {
                    last_newline_pos = Some(i);
                }
            }
            i += 1;
        }

        if let Some(last_nl) = last_newline_pos {
            self.generated_line += newline_count;
            // Column is the number of bytes/chars after the last newline
            let after_last_newline = &new_bytes[last_nl + 1..];
            if after_last_newline.iter().all(u8::is_ascii) {
                self.generated_column = after_last_newline.len() as u32;
            } else {
                let s = std::str::from_utf8(after_last_newline).unwrap_or("");
                self.generated_column = s.encode_utf16().count() as u32;
            }
        } else {
            // No newlines — just advance the column
            if new_bytes.iter().all(u8::is_ascii) {
                self.generated_column += new_bytes.len() as u32;
            } else {
                let s = std::str::from_utf8(new_bytes).unwrap_or("");
                self.generated_column += s.encode_utf16().count() as u32;
            }
        }

        self.last_generated_update = output.len();
    }

    /// Compute line start byte offsets for the source text.
    #[expect(clippy::cast_possible_truncation)]
    fn compute_line_starts(source: &str) -> Vec<u32> {
        let mut starts = vec![0u32];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                starts.push((i + 1) as u32);
            } else if b == b'\r' {
                if source.as_bytes().get(i + 1) == Some(&b'\n') {
                    // \r\n — the \n will push the line start
                    continue;
                }
                starts.push((i + 1) as u32);
            }
        }
        starts
    }
}

/// Compose two sourcemaps: `final_code → intermediate_code` (phase2) and
/// `intermediate_code → original_source` (phase1), producing `final_code → original_source`.
///
/// This is used to chain the TypeScript stripping pass's sourcemap with the
/// Astro codegen's sourcemap.
pub fn remap_sourcemap(
    phase2_map: &oxc_sourcemap::SourceMap,
    phase1_map: &oxc_sourcemap::SourceMap,
    source_path: &str,
    source_content: &str,
) -> oxc_sourcemap::SourceMap {
    let lookup = phase1_map.generate_lookup_table();

    let mut builder = oxc_sourcemap::SourceMapBuilder::default();
    let source_id = builder.set_source_and_content(source_path, source_content);

    for token in phase2_map.get_tokens() {
        // token maps: (dst_line, dst_col) in final code → (src_line, src_col) in intermediate code
        // We look up (src_line, src_col) in phase1_map to find the original .astro position
        let intermediate_line = token.get_src_line();
        let intermediate_col = token.get_src_col();

        if let Some(original_token) =
            phase1_map.lookup_token(&lookup, intermediate_line, intermediate_col)
        {
            // Found a mapping in phase1: intermediate → original .astro source
            let name_id = original_token
                .get_name_id()
                .and_then(|id| phase1_map.get_name(id))
                .map(|name| builder.add_name(name));

            builder.add_token(
                token.get_dst_line(),
                token.get_dst_col(),
                original_token.get_src_line(),
                original_token.get_src_col(),
                Some(source_id),
                name_id,
            );
        }
        // If no mapping found in phase1, we skip this token (generated code with no original source)
    }

    builder.into_sourcemap()
}

/// A sourcemap token used to collect, sort, and deduplicate all mappings
/// before feeding them to the sourcemap builder (which requires tokens in
/// ascending generated-position order).
pub(super) struct RawToken {
    pub dst_line: u32,
    pub dst_col: u32,
    pub src_line: u32,
    pub src_col: u32,
    pub name: Option<String>,
}

/// Shared context for the intermediate-to-final line mapping used by the
/// sourcemap supplementing logic.
pub(super) struct SupplementContext<'a> {
    /// Lines of the intermediate (Phase 1) code.
    pub inter_lines: Vec<&'a str>,
    /// Lines of the final (Phase 2) code.
    pub final_lines: Vec<&'a str>,
    /// Line index of `return $$render\`` in the intermediate code.
    pub i_line: usize,
    /// Line index of `return $$render\`` in the final code.
    pub f_line: usize,
    /// Phase-2 anchor index: intermediate line → vec of
    /// `(inter_col, final_line, final_col)`, sorted by `inter_col` ascending.
    pub phase2_anchors: FxHashMap<u32, Vec<(u32, u32, u32)>>,
    /// Per-line column adjustment info for matched lines.
    /// Key: intermediate line index.
    /// Value: `(ws_delta, escape_insert_positions)`.
    pub line_col_info: FxHashMap<usize, (i64, Vec<usize>)>,
    /// Set of `(dst_line, dst_col)` already covered by Phase 2 composition.
    pub composed_positions: FxHashSet<(u32, u32)>,
}

/// Strip TypeScript from `intermediate_code` and compose the resulting
/// sourcemap with the Phase 1 sourcemap from Astro codegen.
///
/// Phase 1 maps intermediate positions → original `.astro` positions.
/// Phase 2 (TypeScript stripping / oxc_codegen re-emit) maps final positions
/// → intermediate positions.  Composition gives us final positions → original
/// `.astro` positions.
///
/// However, Phase 2 only produces tokens at AST node boundaries.  The template
/// literal `$$render\`...\`` is one AST node, so all the fine-grained Phase 1
/// tokens *inside* it are lost during naïve composition.  After composing, this
/// function carries forward Phase 1 tokens that were not covered by Phase 2 by
/// computing line/column adjustments between the intermediate and final code.
///
/// Returns `(final_code, Option<SourceMap>)` where the sourcemap is `None` if
/// no sourcemap was requested.
///
/// # Panics
///
/// Panics if line or column values exceed `u32` or `i64` (impossible in
/// practice for source files).
pub fn strip_and_compose_sourcemaps(
    allocator: &Allocator,
    intermediate_code: &str,
    phase1_sourcemap: Option<AstroSourcemapBuilder<'_>>,
    source_path: &str,
    source_text: &str,
) -> (String, Option<oxc_sourcemap::SourceMap>) {
    let generate_sourcemap = phase1_sourcemap.is_some();
    let (code, phase2_map) = strip_typescript(allocator, intermediate_code, generate_sourcemap);

    let map = if let (Some(phase2_map), Some(phase1_sm)) = (phase2_map, phase1_sourcemap) {
        let phase1_map = phase1_sm.into_sourcemap();
        let composed = remap_sourcemap(&phase2_map, &phase1_map, source_path, source_text);

        // Supplement: carry forward Phase 1 template-literal tokens,
        // but ONLY on lines where the intermediate and final content
        // match (same text, possibly differing only in leading whitespace).
        //
        // Phase 2 (oxc_codegen) reformats expressions inside template
        // literal interpolations (e.g. adds parens, changes indentation),
        // so columns on those lines differ between intermediate and final.
        // Supplementing such lines with Phase 1 column positions produces
        // wrong mappings.
        //
        // However, oxc_codegen commonly adds a leading tab to lines inside
        // function bodies while leaving the rest of the line identical.
        // We detect this case and adjust column offsets accordingly, so
        // Phase 1 tokens on those lines are preserved with correct columns.
        //
        // For lines that truly differ (reformatted expressions, added
        // parens, etc.), composition (Phase 2 → Phase 1 lookup) already
        // provides correct mappings at AST-node granularity.
        let inter_lines: Vec<&str> = intermediate_code.lines().collect();
        let final_lines: Vec<&str> = code.lines().collect();

        let inter_template_line = inter_lines
            .iter()
            .position(|l| l.contains("return $$render`"));
        let final_template_line = final_lines
            .iter()
            .position(|l| l.contains("return $$render`"));

        let composed = if let (Some(i_line), Some(f_line)) =
            (inter_template_line, final_template_line)
        {
            // For each intermediate line, compute the column adjustment
            // needed to map intermediate columns to final columns.
            //
            // - If the lines match exactly: delta is 0.
            // - If they differ only in leading whitespace: uniform delta.
            // - If they differ due to `</` → `<\/` escaping (template
            //   literal safety): leading-ws delta plus per-position
            //   adjustments for each inserted backslash.
            //
            // `None` means the lines genuinely differ and supplementing
            // is not safe.
            //
            // The value is (ws_delta, escape_insert_positions) where
            // escape_insert_positions lists intermediate columns where
            // a `\` was inserted in the final text (empty for exact or
            // whitespace-only matches).
            let line_col_info: FxHashMap<usize, (i64, Vec<usize>)> = (i_line..inter_lines.len())
                .filter_map(|il| {
                    // il >= i_line is guaranteed by the range, so this
                    // never underflows.  f_line + (il - i_line) is the
                    // corresponding final line index.
                    let fl = il - i_line + f_line;
                    if fl >= final_lines.len() {
                        return None;
                    }
                    let i_text = inter_lines[il];
                    let f_text = final_lines[fl];

                    // Fast path: lines are identical.
                    if i_text == f_text {
                        return Some((il, (0i64, Vec::new())));
                    }

                    // Check if lines differ only in leading whitespace
                    // (and/or template literal escaping like <\/ vs </).
                    let i_trimmed = i_text.trim_start();
                    let f_trimmed = f_text.trim_start();

                    // Leading whitespace counts are bounded by line
                    // length which is bounded by file size — always
                    // representable as i64.
                    let i_ws_len = i64::try_from(i_text.len() - i_trimmed.len())
                        .expect("whitespace length exceeds i64");
                    let f_ws_len = i64::try_from(f_text.len() - f_trimmed.len())
                        .expect("whitespace length exceeds i64");
                    let ws_delta = f_ws_len - i_ws_len;

                    if i_trimmed == f_trimmed {
                        return Some((il, (ws_delta, Vec::new())));
                    }

                    // Normalize template literal escapes: oxc_codegen
                    // escapes `</` as `<\/` inside template literals for
                    // HTML safety.  We treat these as matching, but track
                    // the insertion positions for column adjustment.
                    let i_norm = i_trimmed.replace("<\\/", "</");
                    let f_norm = f_trimmed.replace("<\\/", "</");
                    if i_norm != f_norm {
                        return None; // Content genuinely differs.
                    }

                    // Find positions in the intermediate trimmed text
                    // where `</` occurs — these correspond to `<\/` in
                    // the final text, meaning a `\` was inserted after
                    // the `<`.  We record the intermediate column (in
                    // the full line, not trimmed) of the `/` char, which
                    // is where the shift starts.
                    //
                    // We search `i_trimmed` (intermediate) for `</`
                    // rather than `f_trimmed` for `<\/`, because the
                    // intermediate positions are what we need and
                    // searching the final text would give wrong offsets
                    // for the 2nd+ escape on the same line (each `<\/`
                    // is 3 chars in final but only 2 in intermediate,
                    // causing a cumulative +1 error per prior escape).
                    let mut escape_positions = Vec::new();
                    let i_ws = i_text.len() - i_trimmed.len();
                    let mut search_start = 0;
                    let search_text = i_trimmed;
                    while let Some(pos) = search_text[search_start..].find("</") {
                        let trimmed_pos = search_start + pos;
                        // The `\` is inserted at trimmed_pos + 1 (after `<`).
                        // In intermediate line coords, this corresponds to
                        // i_ws + trimmed_pos + 1 (the `/` in intermediate).
                        escape_positions.push(i_ws + trimmed_pos + 1);
                        search_start = trimmed_pos + 2; // skip past `</`
                    }

                    Some((il, (ws_delta, escape_positions)))
                })
                .collect();

            // Collect the composed tokens into a set of (dst_line, dst_col)
            // so we can skip Phase 1 tokens that were already covered.
            let composed_positions: FxHashSet<(u32, u32)> = composed
                .get_tokens()
                .map(|t| (t.get_dst_line(), t.get_dst_col()))
                .collect();

            // Build Phase-2 anchor index for DIFFER lines.
            //
            // Phase 2 tokens provide (inter_col → final_line, final_col)
            // pairs.  On DIFFER lines (where Phase 2 reformatted JS
            // expressions but left template quasi text intact), we can use
            // the nearest anchor to compute the column offset for Phase-1
            // tokens that sit inside quasi text regions.
            //
            // Keyed by intermediate line → vec of (inter_col, final_line,
            // final_col), sorted by inter_col ascending.
            let phase2_anchors: FxHashMap<u32, Vec<(u32, u32, u32)>> = {
                let mut map: FxHashMap<u32, Vec<(u32, u32, u32)>> = FxHashMap::default();
                for t in phase2_map.get_tokens() {
                    map.entry(t.get_src_line()).or_default().push((
                        t.get_src_col(),
                        t.get_dst_line(),
                        t.get_dst_col(),
                    ));
                }
                for v in map.values_mut() {
                    v.sort_by_key(|&(ic, _, _)| ic);
                }
                map
            };

            let ctx = SupplementContext {
                inter_lines,
                final_lines,
                i_line,
                f_line,
                phase2_anchors,
                line_col_info,
                composed_positions,
            };

            let mut all_tokens: Vec<RawToken> = Vec::new();

            // 1. Existing composed tokens.
            for t in composed.get_tokens() {
                let name = t
                    .get_name_id()
                    .and_then(|id| composed.get_name(id))
                    .map(std::string::ToString::to_string);
                all_tokens.push(RawToken {
                    dst_line: t.get_dst_line(),
                    dst_col: t.get_dst_col(),
                    src_line: t.get_src_line(),
                    src_col: t.get_src_col(),
                    name,
                });
            }

            // 2. Phase 1 tokens inside the template literal, on lines
            //    where the content matches (exactly or after leading
            //    whitespace adjustment), OR on DIFFER lines where the
            //    token sits inside a quasi text region that is identical
            //    between intermediate and final code.
            supplement_phase1_tokens(&phase1_map, &ctx, &mut all_tokens);

            // Sort by generated position (line first, then column).
            all_tokens.sort_by(|a, b| a.dst_line.cmp(&b.dst_line).then(a.dst_col.cmp(&b.dst_col)));

            // Deduplicate tokens at the same generated position.
            all_tokens.dedup_by(|a, b| a.dst_line == b.dst_line && a.dst_col == b.dst_col);

            // Build the final sourcemap in sorted order.
            let mut builder = oxc_sourcemap::SourceMapBuilder::default();
            let src_id = builder.set_source_and_content(source_path, source_text);

            for t in &all_tokens {
                let name_id = t.name.as_deref().map(|n| builder.add_name(n));
                builder.add_token(
                    t.dst_line,
                    t.dst_col,
                    t.src_line,
                    t.src_col,
                    Some(src_id),
                    name_id,
                );
            }

            builder.into_sourcemap()
        } else {
            composed
        };

        Some(composed)
    } else {
        None
    };

    (code, map)
}

/// Carry forward Phase 1 tokens inside the template literal region that
/// were not covered by Phase 2 composition.
///
/// For matched lines (exact, leading-whitespace, or escape-normalized), the
/// token column is adjusted by the whitespace delta and escape shift.
///
/// For DIFFER lines (where Phase 2 reformatted JS expressions), tokens in
/// quasi text regions are carried forward using the nearest Phase-2 anchor
/// to compute the column offset, with a text-verification check.
///
/// For lines with no Phase-2 anchors (pure template quasi text), a small
/// window search finds the matching final line.
fn supplement_phase1_tokens(
    phase1_map: &oxc_sourcemap::SourceMap,
    ctx: &SupplementContext<'_>,
    all_tokens: &mut Vec<RawToken>,
) {
    for t in phase1_map.get_tokens() {
        let gen_line = t.get_dst_line() as usize;
        // Only consider tokens at or after the template start.
        if gen_line < ctx.i_line {
            continue;
        }

        let token_col = t.get_dst_col();

        // Try the matched-line path first (exact, leading-ws,
        // or escape-normalized match).  If absent, fall through
        // to anchor-based supplementing for DIFFER lines.
        let (adjusted_line, adjusted_col) =
            if let Some((ws_delta, escape_positions)) = ctx.line_col_info.get(&gen_line) {
                // gen_line >= i_line (checked above), so this
                // never underflows.
                let al = u32::try_from(gen_line - ctx.i_line + ctx.f_line)
                    .expect("adjusted line exceeds u32");

                // Count how many escape insertions occur at or
                // before this token's column.  Each `<\/` escape
                // adds 1 byte (the `\`) that shifts subsequent
                // columns.
                let tc = token_col as usize;
                let escape_shift =
                    i64::try_from(escape_positions.iter().filter(|&&pos| pos <= tc).count())
                        .expect("escape count exceeds i64");

                let ac = u32::try_from((i64::from(token_col) + ws_delta + escape_shift).max(0))
                    .expect("adjusted column exceeds u32");
                (al, ac)
            } else if let Some(result) = try_anchor_supplement(token_col, gen_line, ctx) {
                result
            } else {
                continue;
            };

        // Skip if already covered by composition.
        if ctx
            .composed_positions
            .contains(&(adjusted_line, adjusted_col))
        {
            continue;
        }

        // Skip if beyond final code.
        if (adjusted_line as usize) >= ctx.final_lines.len() {
            continue;
        }

        let name = t
            .get_name_id()
            .and_then(|id| phase1_map.get_name(id))
            .map(std::string::ToString::to_string);

        all_tokens.push(RawToken {
            dst_line: adjusted_line,
            dst_col: adjusted_col,
            src_line: t.get_src_line(),
            src_col: t.get_src_col(),
            name,
        });
    }
}

/// Try to supplement a Phase 1 token on a DIFFER line using Phase-2 anchors,
/// or by searching a window of final lines for pure quasi text.
///
/// Returns `Some((final_line, final_col))` if supplementing is possible,
/// `None` if the token should be skipped.
fn try_anchor_supplement(
    token_col: u32,
    gen_line: usize,
    ctx: &SupplementContext<'_>,
) -> Option<(u32, u32)> {
    let gen_line_u32 = u32::try_from(gen_line).ok()?;

    if let Some(anchors) = ctx.phase2_anchors.get(&gen_line_u32) {
        // Find the nearest anchor with inter_col <= token_col
        // (the last one in sorted order that doesn't exceed it).
        let nearest = anchors.iter().rev().find(|&&(ic, _, _)| ic <= token_col);

        if let Some(&(anchor_ic, anchor_fl, anchor_fc)) = nearest {
            let delta = i64::from(anchor_fc) - i64::from(anchor_ic);
            let candidate_col = u32::try_from((i64::from(token_col) + delta).max(0))
                .expect("candidate col exceeds u32");

            // Verify: the text at the candidate final position must match the
            // intermediate text at the token position.  This confirms we are in
            // a quasi region (not a reformatted expression).
            let i_text = ctx.inter_lines.get(gen_line).copied().unwrap_or("");
            let f_text = ctx
                .final_lines
                .get(anchor_fl as usize)
                .copied()
                .unwrap_or("");
            let tc = token_col as usize;
            let cc = candidate_col as usize;
            // Use a short verification window; 2 bytes is enough to confirm
            // we're at the same quasi text (e.g. "<p", "</", etc.).
            let verify_len = 2;
            let text_matches = tc + verify_len <= i_text.len()
                && cc + verify_len <= f_text.len()
                && i_text.as_bytes()[tc..tc + verify_len] == f_text.as_bytes()[cc..cc + verify_len];

            if text_matches {
                return Some((anchor_fl, candidate_col));
            }
        }
        // No anchor at or before token_col (or text mismatch): the token is in
        // a quasi text region that starts before all Phase-2 anchors on this
        // line (e.g. `<span>` at col 0 when the JS suffix starts at col 25).
        // Fall through to the window search below.
    }

    {
        // Either: no Phase-2 anchors exist for this intermediate line (pure
        // quasi text), or all anchors are after the token's column (token is
        // in a quasi region before the first interpolation on this line).
        //
        // Phase-2 reformatting may shift these lines by inserting extra lines
        // for object literal properties, but the text content remains
        // identical in the quasi regions.
        //
        // Search a window of final lines around the expected position for
        // a line containing the same text at the token's column.
        let i_text = ctx.inter_lines.get(gen_line).copied().unwrap_or("");
        let tc = token_col as usize;
        let verify_len = 2;
        if tc + verify_len > i_text.len() {
            return None;
        }
        let needle = &i_text.as_bytes()[tc..tc + verify_len];

        // Expected final line based on the template offset.
        // gen_line >= i_line (checked by caller).
        let expected_fl = gen_line - ctx.i_line + ctx.f_line;
        // Search ±5 lines around expected position to account for
        // Phase-2 reformatting inserting a few extra lines.
        let search_start = expected_fl.saturating_sub(5);
        let search_end = (expected_fl + 6).min(ctx.final_lines.len());

        for (fl, f_text) in ctx
            .final_lines
            .iter()
            .enumerate()
            .take(search_end)
            .skip(search_start)
        {
            // Check same column (the text is quasi literal,
            // so the column should be identical).
            if tc + verify_len <= f_text.len() && &f_text.as_bytes()[tc..tc + verify_len] == needle
            {
                let fl_u32 = u32::try_from(fl).expect("final line exceeds u32");
                return Some((fl_u32, token_col));
            }
            // Also check with leading whitespace adjustment:
            // Phase-2 may have added or removed leading ws.
            let f_trimmed = f_text.trim_start();
            let i_trimmed = i_text.trim_start();
            let f_ws = i64::try_from(f_text.len() - f_trimmed.len()).unwrap_or(0);
            let i_ws = i64::try_from(i_text.len() - i_trimmed.len()).unwrap_or(0);
            let ws_adj = f_ws - i_ws;
            let adj_col_i64 = (i64::from(token_col) + ws_adj).max(0);
            let adj_col = usize::try_from(adj_col_i64).expect("adjusted column exceeds usize");
            if adj_col + verify_len <= f_text.len()
                && &f_text.as_bytes()[adj_col..adj_col + verify_len] == needle
            {
                let fl_u32 = u32::try_from(fl).expect("final line exceeds u32");
                let ac = u32::try_from(adj_col).expect("adjusted col exceeds u32");
                return Some((fl_u32, ac));
            }
        }

        None
    }
}

/// Strip TypeScript syntax from generated code.
///
/// Parses the code as TypeScript, runs `oxc_transformer` (TS-only stripping,
/// no JSX transform, no ES downleveling), and re-emits as JavaScript.
fn strip_typescript(
    allocator: &Allocator,
    code: &str,
    generate_sourcemap: bool,
) -> (String, Option<oxc_sourcemap::SourceMap>) {
    let source_type = oxc_span::SourceType::mjs().with_typescript(true);
    let ret = oxc_parser::Parser::new(allocator, code, source_type).parse();

    if !ret.errors.is_empty() {
        // If parsing fails, return the code unchanged — the downstream
        // consumer will report a better error.
        return (code.to_string(), None);
    }

    let mut program = ret.program;
    let scoping = oxc_semantic::SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .build(&program)
        .semantic
        .into_scoping();

    let options = oxc_transformer::TransformOptions::default();
    let _ = oxc_transformer::Transformer::new(allocator, std::path::Path::new(""), &options)
        .build_with_scoping(scoping, &mut program);

    let codegen_options = CodegenOptions {
        single_quote: false,
        source_map_path: if generate_sourcemap {
            Some(std::path::PathBuf::from("intermediate.js"))
        } else {
            None
        },
        ..CodegenOptions::default()
    };
    let result = oxc_codegen::Codegen::new()
        .with_options(codegen_options)
        .build(&program);
    (result.code, result.map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_line_starts() {
        let source = "line1\nline2\nline3";
        let starts = AstroSourcemapBuilder::compute_line_starts(source);
        assert_eq!(starts, vec![0, 6, 12]);
    }

    #[test]
    fn test_compute_line_starts_crlf() {
        let source = "line1\r\nline2\r\nline3";
        let starts = AstroSourcemapBuilder::compute_line_starts(source);
        assert_eq!(starts, vec![0, 7, 14]);
    }

    #[test]
    fn test_byte_offset_to_line_column() {
        let source = "abc\ndef\nghi";
        let builder = AstroSourcemapBuilder::new(Path::new("test.astro"), source);

        // 'a' at offset 0 → line 0, col 0
        assert_eq!(builder.byte_offset_to_line_column(0), (0, 0));
        // 'c' at offset 2 → line 0, col 2
        assert_eq!(builder.byte_offset_to_line_column(2), (0, 2));
        // 'd' at offset 4 → line 1, col 0
        assert_eq!(builder.byte_offset_to_line_column(4), (1, 0));
        // 'g' at offset 8 → line 2, col 0
        assert_eq!(builder.byte_offset_to_line_column(8), (2, 0));
        // 'i' at offset 10 → line 2, col 2
        assert_eq!(builder.byte_offset_to_line_column(10), (2, 2));
    }

    #[test]
    fn test_basic_mapping() {
        let source = "hello\nworld";
        let mut builder = AstroSourcemapBuilder::new(Path::new("test.astro"), source);

        let output = b"const x = 'hello';\n";
        builder.add_source_mapping(output, 0);

        let map = builder.into_sourcemap();
        let tokens: Vec<_> = map.get_tokens().collect();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].get_src_line(), 0);
        assert_eq!(tokens[0].get_src_col(), 0);
    }
}
