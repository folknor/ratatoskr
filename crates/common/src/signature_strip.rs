//! Signature stripping and quote collapsing for message bodies.
//!
//! Two independent transforms intended for "chat-style" rendering where
//! signatures and quoted reply chains are noise.
//!
//! Layered, conservative-first:
//!
//! - Layer 1 (HTML mode only): known client signature wrappers
//!   (`gmail_signature`, `moz-cite-prefix`).
//! - Layer 2: RFC 3676 `-- ` separator.
//! - Layer 3: caller-supplied user signatures, exact-suffix match after
//!   trimming. Pass these for outbound messages only.
//!
//! Phase 5 layers (per-sender learned suffixes, heuristic valedictions)
//! are intentionally not implemented here yet.
//!
//! Both functions are non-destructive: the caller keeps the original body
//! and only uses the return value for display. If a transform leaves an
//! empty result the caller should fall back to the original.

/// Strip the trailing signature block from a message body.
///
/// `is_html` selects between HTML and plain-text mode. `user_signatures`
/// are tried as exact suffixes (after trim) for Layer 3 - pass an empty
/// slice when not applicable (e.g. inbound messages).
pub fn strip_signature(body: &str, is_html: bool, user_signatures: &[&str]) -> String {
    if is_html {
        return strip_html_signature(body);
    }
    let after_delim = strip_rfc3676_delimiter(body);
    strip_user_signature_suffix(&after_delim, user_signatures)
}

/// Collapse quoted reply content from a message body.
///
/// Plain-text mode strips a trailing `On <date>, <person> wrote:` line
/// and any block of `>`-prefixed lines that follow. HTML mode removes
/// `gmail_quote`, `gmail_extra`, `yahoo_quoted` wrappers and
/// `<blockquote type="cite">` blocks.
pub fn collapse_quotes(body: &str, is_html: bool) -> String {
    if is_html {
        return collapse_html_quotes(body);
    }
    collapse_text_quotes(body)
}

// ── Plain-text ────────────────────────────────────────────

fn strip_rfc3676_delimiter(body: &str) -> String {
    let mut consumed = 0;
    for line in body.split_inclusive('\n') {
        let no_eol = line.trim_end_matches('\n').trim_end_matches('\r');
        if no_eol == "-- " {
            return body[..consumed].trim_end().to_string();
        }
        consumed += line.len();
    }
    body.to_string()
}

fn strip_user_signature_suffix(body: &str, user_signatures: &[&str]) -> String {
    let trimmed_end_len = body.trim_end().len();
    let body_check = &body[..trimmed_end_len];
    for sig in user_signatures {
        let sig = sig.trim();
        if sig.is_empty() {
            continue;
        }
        if body_check.ends_with(sig) {
            let cut = trimmed_end_len - sig.len();
            return body[..cut].trim_end().to_string();
        }
    }
    body.to_string()
}

fn collapse_text_quotes(body: &str) -> String {
    // Track two cut candidates while walking forward:
    //   - on_wrote_cut: the byte offset of an "On ... wrote:" header
    //   - quote_block_start: the byte offset of the start of a contiguous
    //     run of `>`-prefixed lines that reach the end of the body
    //
    // The on-wrote header is more specific and wins when both are found.
    // A quote-block candidate is invalidated by any non-empty, non-quoted
    // line (means there's actual content after the quote block).
    let mut on_wrote_cut: Option<usize> = None;
    let mut quote_block_start: Option<usize> = None;
    let mut consumed = 0;

    for line in body.split_inclusive('\n') {
        let no_eol = line.trim_end_matches('\n').trim_end_matches('\r');
        let trimmed = no_eol.trim();

        if is_on_wrote_line(trimmed) {
            on_wrote_cut = Some(consumed);
            break;
        }

        if trimmed.starts_with('>') {
            if quote_block_start.is_none() {
                quote_block_start = Some(consumed);
            }
        } else if !trimmed.is_empty() {
            quote_block_start = None;
        }

        consumed += line.len();
    }

    let cut = on_wrote_cut.or(quote_block_start);
    match cut {
        Some(c) => body[..c].trim_end().to_string(),
        None => body.to_string(),
    }
}

/// Recognise a Gmail-style "On <date>, <person> wrote:" attribution line.
///
/// Phase 2 only handles the English form. Other locales (French
/// `Le ... a écrit :`, German `Am ... schrieb ...:`) and Outlook's
/// header-block format are deferred to later phases.
fn is_on_wrote_line(line: &str) -> bool {
    if !line.starts_with("On ") {
        return false;
    }
    line.ends_with("wrote:") || line.ends_with("wrote :")
}

// ── HTML ──────────────────────────────────────────────────

fn strip_html_signature(body: &str) -> String {
    use lol_html::{RewriteStrSettings, element, rewrite_str};

    let settings = RewriteStrSettings {
        element_content_handlers: vec![
            // Gmail's appended signature wrapper.
            element!("div.gmail_signature", |el| {
                el.remove();
                Ok(())
            }),
            // Thunderbird's "On X, Y wrote:" prefix line element. The
            // following <blockquote> is a quote, handled by collapse_quotes.
            element!("div.moz-cite-prefix", |el| {
                el.remove();
                Ok(())
            }),
        ],
        ..RewriteStrSettings::default()
    };
    match rewrite_str(body, settings) {
        Ok(out) => out,
        Err(e) => {
            log::warn!("strip_html_signature lol_html rewrite failed: {e}");
            body.to_string()
        }
    }
}

fn collapse_html_quotes(body: &str) -> String {
    use lol_html::{RewriteStrSettings, element, rewrite_str};

    let settings = RewriteStrSettings {
        element_content_handlers: vec![
            element!("div.gmail_quote", |el| {
                el.remove();
                Ok(())
            }),
            element!("div.gmail_extra", |el| {
                el.remove();
                Ok(())
            }),
            element!("div.yahoo_quoted", |el| {
                el.remove();
                Ok(())
            }),
            element!(r#"blockquote[type="cite"]"#, |el| {
                el.remove();
                Ok(())
            }),
        ],
        ..RewriteStrSettings::default()
    };
    match rewrite_str(body, settings) {
        Ok(out) => out,
        Err(e) => {
            log::warn!("collapse_html_quotes lol_html rewrite failed: {e}");
            body.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{collapse_quotes, strip_signature};

    #[test]
    fn strips_rfc3676_delimiter() {
        let body = "yeah that works\n-- \nJohn Smith\nAcme Corp";
        let out = strip_signature(body, false, &[]);
        assert_eq!(out, "yeah that works");
    }

    #[test]
    fn passes_through_when_no_delimiter() {
        let body = "just a message\nwith two lines";
        assert_eq!(strip_signature(body, false, &[]), body);
    }

    #[test]
    fn dash_dash_without_trailing_space_is_not_a_delimiter() {
        // RFC 3676 specifies "-- " (with trailing space). Without the
        // space it's just two dashes - don't strip.
        let body = "yeah that works\n--\nJohn Smith";
        let out = strip_signature(body, false, &[]);
        assert_eq!(out, body);
    }

    #[test]
    fn user_signature_exact_suffix_strips() {
        let sig = "Best,\nAtle";
        let body = "see attached\n\nBest,\nAtle";
        let out = strip_signature(body, false, &[sig]);
        assert_eq!(out, "see attached");
    }

    #[test]
    fn user_signature_no_match_passes_through() {
        let sig = "Cheers,\nMallory";
        let body = "see attached\n\nBest,\nAtle";
        assert_eq!(strip_signature(body, false, &[sig]), body);
    }

    #[test]
    fn collapses_on_wrote_attribution_and_blockquote() {
        let body = "yeah that works\n\nOn Mon, Mar 25, 2026 at 9:14 AM Alice <alice@example.com> wrote:\n> sounds good\n> let me know";
        let out = collapse_quotes(body, false);
        assert_eq!(out, "yeah that works");
    }

    #[test]
    fn collapses_trailing_quote_block_without_attribution() {
        let body = "ok\n> previous line\n> another";
        let out = collapse_quotes(body, false);
        assert_eq!(out, "ok");
    }

    #[test]
    fn keeps_content_after_quote_block() {
        // A quote block in the middle of the body shouldn't be cut -
        // there's real content after.
        let body = "before\n> quoted\n> stuff\nactual reply";
        assert_eq!(collapse_quotes(body, false), body);
    }

    #[test]
    fn html_strips_gmail_signature() {
        let body = r#"<p>yeah that works</p><div class="gmail_signature">--<br>John</div>"#;
        let out = strip_signature(body, true, &[]);
        assert!(!out.contains("gmail_signature"), "got: {out}");
        assert!(out.contains("yeah that works"), "got: {out}");
    }

    #[test]
    fn html_collapses_gmail_quote() {
        let body = r#"<p>ok</p><div class="gmail_quote">earlier</div>"#;
        let out = collapse_quotes(body, true);
        assert!(!out.contains("gmail_quote"), "got: {out}");
        assert!(out.contains("ok"), "got: {out}");
    }
}
