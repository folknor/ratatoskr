    use super::*;
    use super::dates::naive_date_to_timestamp;

    // -- Basic operator parsing --

    #[test]
    fn parses_simple_unread() {
        let q = parse_query("is:unread");
        assert_eq!(q.is_unread, Some(true));
        assert!(q.free_text.is_empty());
    }

    #[test]
    fn parses_from_with_free_text() {
        let q = parse_query("hello from:alice world");
        assert_eq!(q.from, vec!["alice"]);
        assert_eq!(q.free_text, "hello world");
    }

    #[test]
    fn parses_quoted_value() {
        let q = parse_query("from:\"John Doe\"");
        assert_eq!(q.from, vec!["John Doe"]);
    }

    #[test]
    fn parses_has_attachment() {
        let q = parse_query("has:attachment");
        assert!(q.has_attachment);
    }

    #[test]
    fn parses_date_filters() {
        let q = parse_query("after:2024/01/01 before:2024/12/31");
        assert!(q.after.is_some());
        assert!(q.before.is_some());
    }

    #[test]
    fn parses_label() {
        let q = parse_query("label:Important");
        assert_eq!(q.label, vec!["Important"]);
    }

    #[test]
    fn parses_multiple_operators() {
        let q = parse_query("is:unread is:starred from:bob has:attachment");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.is_starred, Some(true));
        assert_eq!(q.from, vec!["bob"]);
        assert!(q.has_attachment);
    }

    #[test]
    fn handles_case_insensitive_operators() {
        let q = parse_query("IS:Unread FROM:Alice");
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.from, vec!["Alice"]);
    }

    #[test]
    fn parses_extended_is_values() {
        let q = parse_query("is:snoozed");
        assert_eq!(q.is_snoozed, Some(true));

        let q = parse_query("is:pinned");
        assert_eq!(q.is_pinned, Some(true));

        let q = parse_query("is:muted");
        assert_eq!(q.is_muted, Some(true));
    }

    #[test]
    fn date_with_dashes() {
        let q = parse_query("after:2024-06-15");
        assert!(q.after.is_some());
    }

    // -- OR semantics --

    #[test]
    fn or_semantics_from() {
        let q = parse_query("from:alice from:bob");
        assert_eq!(q.from, vec!["alice", "bob"]);
    }

    #[test]
    fn or_semantics_to() {
        let q = parse_query("to:alice to:bob to:carol");
        assert_eq!(q.to, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn or_semantics_label() {
        let q = parse_query("label:Work label:Personal");
        assert_eq!(q.label, vec!["Work", "Personal"]);
    }

    // -- New operators --

    #[test]
    fn parses_account_operator() {
        let q = parse_query("account:work");
        assert_eq!(q.account, vec!["work"]);
    }

    #[test]
    fn parses_folder_operator() {
        let q = parse_query("folder:Inbox");
        assert_eq!(q.folder, vec!["Inbox"]);
    }

    #[test]
    fn parses_in_operator() {
        let q = parse_query("in:inbox");
        assert_eq!(q.in_folder, vec!["inbox"]);
    }

    #[test]
    fn parses_is_tagged() {
        let q = parse_query("is:tagged");
        assert_eq!(q.is_tagged, Some(true));
    }

    #[test]
    fn parses_has_contact() {
        let q = parse_query("has:contact");
        assert!(q.has_contact);
    }

    #[test]
    fn parses_type_operator() {
        let q = parse_query("type:application/pdf");
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
    }

    // -- has: expansion --

    #[test]
    fn has_pdf_expansion() {
        let q = parse_query("has:pdf");
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
    }

    #[test]
    fn has_image_expansion() {
        let q = parse_query("has:image");
        assert_eq!(q.attachment_types.len(), 5);
        assert!(q.attachment_types.contains(&"image/jpeg".to_owned()));
        assert!(q.attachment_types.contains(&"image/png".to_owned()));
    }

    #[test]
    fn has_excel_expansion() {
        let q = parse_query("has:excel");
        assert_eq!(q.attachment_types.len(), 4);
        assert!(q.attachment_types.contains(&"text/csv".to_owned()));
    }

    #[test]
    fn has_spreadsheet_alias() {
        let q_excel = parse_query("has:excel");
        let q_spreadsheet = parse_query("has:spreadsheet");
        assert_eq!(q_excel.attachment_types, q_spreadsheet.attachment_types);
    }

    #[test]
    fn has_document_union() {
        let q = parse_query("has:document");
        // Should contain word types + pdf.
        assert!(
            q.attachment_types
                .contains(&"application/msword".to_owned())
        );
        assert!(q.attachment_types.contains(&"application/pdf".to_owned()));
        assert!(q.attachment_types.contains(&"application/rtf".to_owned()));
    }

    #[test]
    fn has_archive_expansion() {
        let q = parse_query("has:archive");
        assert!(q.attachment_types.contains(&"application/zip".to_owned()));
        assert!(q.attachment_types.contains(&"application/gzip".to_owned()));
    }

    #[test]
    fn has_video_expansion() {
        let q = parse_query("has:video");
        assert_eq!(q.attachment_types, vec!["video/*"]);
    }

    #[test]
    fn has_audio_expansion() {
        let q = parse_query("has:audio");
        assert_eq!(q.attachment_types, vec!["audio/*"]);
    }

    #[test]
    fn has_calendar_expansion() {
        let q = parse_query("has:calendar");
        assert!(q.attachment_types.contains(&"text/calendar".to_owned()));
        assert!(q.attachment_types.contains(&"application/ics".to_owned()));
    }

    #[test]
    fn has_powerpoint_expansion() {
        let q = parse_query("has:powerpoint");
        assert_eq!(q.attachment_types.len(), 3);
    }

    // -- Date parsing --

    #[test]
    fn date_relative_offset_negative() {
        let q = parse_query("after:-7");
        assert!(q.after.is_some());
        // Should be 7 days ago at start of day.
        let today = chrono::Local::now().date_naive();
        let expected = today - chrono::Duration::days(7);
        let expected_ts = naive_date_to_timestamp(expected);
        assert_eq!(q.after.map(DateBound::timestamp), expected_ts);
    }

    #[test]
    fn date_relative_offset_zero() {
        let q = parse_query("after:0");
        assert!(q.after.is_some());
        let today = chrono::Local::now().date_naive();
        let expected_ts = naive_date_to_timestamp(today);
        assert_eq!(q.after.map(DateBound::timestamp), expected_ts);
    }

    #[test]
    fn date_year_only() {
        let q = parse_query("after:2025");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2025, 1, 1).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_year_month_compact() {
        let q = parse_query("after:202603");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_full_compact() {
        let q = parse_query("after:20260311");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 11).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_slash_separated() {
        let q = parse_query("before:2026/03/11");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 11).and_then(naive_date_to_timestamp);
        assert_eq!(q.before.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_dash_separated() {
        let q = parse_query("before:2026-03-11");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 11).and_then(naive_date_to_timestamp);
        assert_eq!(q.before.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_space_separated_greedy() {
        let q = parse_query("after:2026 03 11");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 11).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
    }

    #[test]
    fn date_space_separated_year_month_only() {
        let q = parse_query("after:2026 03 hello");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
        assert_eq!(q.free_text, "hello");
    }

    #[test]
    fn date_space_greedy_does_not_consume_non_digits() {
        let q = parse_query("after:2026 hello");
        let expected =
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).and_then(naive_date_to_timestamp);
        assert_eq!(q.after.map(DateBound::timestamp), expected);
        assert_eq!(q.free_text, "hello");
    }

    // -- has_any_operator helper --

    #[test]
    fn has_any_operator_empty() {
        let q = ParsedQuery::default();
        assert!(!q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_from() {
        let q = parse_query("from:alice");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_free_text_only() {
        let q = parse_query("hello world");
        assert!(!q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_flags() {
        let q = parse_query("is:unread");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_date() {
        let q = parse_query("after:2024/01/01");
        assert!(q.has_any_operator());
    }

    #[test]
    fn has_any_operator_with_attachment() {
        let q = parse_query("has:attachment");
        assert!(q.has_any_operator());
    }

    // -- Free text extraction with new operators --

    #[test]
    fn free_text_with_new_operators() {
        let q = parse_query("hello account:work folder:Inbox in:sent world");
        assert_eq!(q.free_text, "hello world");
        assert_eq!(q.account, vec!["work"]);
        assert_eq!(q.folder, vec!["Inbox"]);
        assert_eq!(q.in_folder, vec!["sent"]);
    }

    #[test]
    fn complex_query_with_many_operators() {
        let q = parse_query(
            "meeting notes from:alice from:bob label:Work is:unread has:pdf account:personal",
        );
        assert_eq!(q.free_text, "meeting notes");
        assert_eq!(q.from, vec!["alice", "bob"]);
        assert_eq!(q.label, vec!["Work"]);
        assert_eq!(q.is_unread, Some(true));
        assert_eq!(q.attachment_types, vec!["application/pdf"]);
        assert_eq!(q.account, vec!["personal"]);
    }

    // -- Removed operators should not parse --

    #[test]
    fn subject_not_parsed_as_operator() {
        let q = parse_query("subject:meeting");
        // "subject:" is not an operator, so it becomes free text.
        assert_eq!(q.free_text, "subject:meeting");
    }

    #[test]
    fn is_important_falls_back_to_free_text() {
        let q = parse_query("is:important");
        // "important" is not a recognized is: value, so the original token
        // is preserved as free text for full-text search.
        assert_eq!(q.free_text, "is:important");
        // Verify no structured flags were set.
        assert!(!q.has_any_operator());
    }

    // -- Cursor context analysis --

    #[test]
    fn cursor_in_free_text() {
        let ctx = analyze_cursor_context("hello world", 5);
        assert_eq!(ctx, CursorContext::FreeText);
    }

    #[test]
    fn cursor_at_operator_value_start() {
        let ctx = analyze_cursor_context("from:", 5);
        assert_eq!(
            ctx,
            CursorContext::InsideOperator {
                operator: "from".to_owned(),
                partial_value: String::new(),
                value_start: 5,
                value_end: 5,
            }
        );
    }

    #[test]
    fn cursor_inside_operator_value() {
        let ctx = analyze_cursor_context("from:ali", 8);
        assert_eq!(
            ctx,
            CursorContext::InsideOperator {
                operator: "from".to_owned(),
                partial_value: "ali".to_owned(),
                value_start: 5,
                value_end: 8,
            }
        );
    }

    #[test]
    fn cursor_after_completed_operator() {
        let ctx = analyze_cursor_context("from:alice ", 11);
        assert_eq!(ctx, CursorContext::FreeText);
    }

    #[test]
    fn cursor_inside_quoted_value() {
        let ctx = analyze_cursor_context("from:\"John D", 12);
        assert_eq!(
            ctx,
            CursorContext::InsideOperator {
                operator: "from".to_owned(),
                partial_value: "John D".to_owned(),
                value_start: 5,
                value_end: 12,
            }
        );
    }

    #[test]
    fn cursor_in_second_operator() {
        let ctx = analyze_cursor_context("from:alice label:wo", 19);
        assert_eq!(
            ctx,
            CursorContext::InsideOperator {
                operator: "label".to_owned(),
                partial_value: "wo".to_owned(),
                value_start: 17,
                value_end: 19,
            }
        );
    }

    #[test]
    fn cursor_case_insensitive_operator() {
        let ctx = analyze_cursor_context("FROM:ali", 8);
        assert_eq!(
            ctx,
            CursorContext::InsideOperator {
                operator: "from".to_owned(),
                partial_value: "ali".to_owned(),
                value_start: 5,
                value_end: 8,
            }
        );
    }
