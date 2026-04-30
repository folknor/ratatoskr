#![allow(dead_code)]

use iced::widget::{column, container, row, text};
use iced::Element;

use crate::ui::layout::TEXT_LG;
use crate::ui::theme;

/// Render a text body with search terms highlighted.
///
/// Splits the body into segments: plain text and highlighted spans.
/// Highlighted spans get a semi-transparent accent background via
/// a container with a background color.
pub(super) fn highlighted_text_body<'a, M: 'a>(
    body: &'a str,
    terms: &'a [String],
) -> Element<'a, M> {
    let lower_body = body.to_lowercase();
    let mut segments: Vec<(usize, usize, bool)> = Vec::new();

    let mut match_ranges: Vec<(usize, usize)> = Vec::new();
    for term in terms {
        let lower_term = term.to_lowercase();
        if lower_term.is_empty() {
            continue;
        }
        let mut start = 0;
        while let Some(pos) = lower_body[start..].find(&lower_term) {
            let abs_start = start + pos;
            let abs_end = abs_start + lower_term.len();
            match_ranges.push((abs_start, abs_end));
            start = abs_end;
        }
    }

    if match_ranges.is_empty() {
        return text(body).size(TEXT_LG).style(text::secondary).into();
    }

    match_ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in match_ranges {
        if let Some(last) = merged.last_mut()
            && s <= last.1
        {
            last.1 = last.1.max(e);
            continue;
        }
        merged.push((s, e));
    }

    let mut pos = 0;
    for (s, e) in &merged {
        if pos < *s {
            segments.push((pos, *s, false));
        }
        segments.push((*s, *e, true));
        pos = *e;
    }
    if pos < body.len() {
        segments.push((pos, body.len(), false));
    }

    let mut col = column![].spacing(0);

    for line in body.split('\n') {
        let line_start = line.as_ptr() as usize - body.as_ptr() as usize;
        let line_end = line_start + line.len();

        let mut line_row = row![].spacing(0);
        let mut has_highlight = false;

        for &(seg_start, seg_end, is_match) in &segments {
            if seg_end <= line_start || seg_start >= line_end {
                continue;
            }
            let s = seg_start.max(line_start) - line_start;
            let e = seg_end.min(line_end) - line_start;
            let segment_text = &line[s..e];
            if segment_text.is_empty() {
                continue;
            }

            if is_match {
                has_highlight = true;
                line_row = line_row.push(
                    container(text(segment_text).size(TEXT_LG).style(text::secondary))
                        .style(theme::ContainerClass::Badge.style()),
                );
            } else {
                line_row = line_row.push(text(segment_text).size(TEXT_LG).style(text::secondary));
            }
        }

        if !has_highlight
            && segments
                .iter()
                .all(|&(s, e, _)| e <= line_start || s >= line_end)
        {
            line_row = line_row.push(text(line).size(TEXT_LG).style(text::secondary));
        }

        col = col.push(line_row);
    }

    col.into()
}
