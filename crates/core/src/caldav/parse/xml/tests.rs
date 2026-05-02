use super::*;

    #[test]
    fn parse_propfind_calendars_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
               xmlns:CS="http://calendarserver.org/ns/"
               xmlns:IC="http://apple.com/ns/ical/">
  <D:response>
    <D:href>/calendars/user/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:displayname>User Calendars</D:displayname>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Personal</D:displayname>
        <CS:getctag>ctag-abc-123</CS:getctag>
        <IC:calendar-color>#0000FFFF</IC:calendar-color>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/work/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Work</D:displayname>
        <CS:getctag>ctag-def-456</CS:getctag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 2);
        assert_eq!(calendars[0].href, "/calendars/user/personal/");
        assert_eq!(calendars[0].display_name.as_deref(), Some("Personal"));
        assert_eq!(calendars[0].ctag.as_deref(), Some("ctag-abc-123"));
        // Apple emits the ARGB form (`#RRGGBBAA`); we fold the alpha at
        // parse time so UI consumers see the same `#RRGGBB` as servers
        // that emit a 7-char value directly.
        assert_eq!(calendars[0].color.as_deref(), Some("#0000FF"));
        assert_eq!(calendars[1].href, "/calendars/user/work/");
        assert_eq!(calendars[1].display_name.as_deref(), Some("Work"));
        assert!(calendars[1].color.is_none());
    }

    #[test]
    fn parse_propfind_events_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"collection-etag"</D:getetag>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/calendar; charset=utf-8</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event2.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-222"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let result = parse_propfind_events(xml);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].uri, "/calendars/user/personal/event1.ics");
        // ETag values are preserved verbatim, including the RFC 7232 quotes.
        assert_eq!(result.entries[0].etag, "\"etag-111\"");
        assert_eq!(result.entries[1].uri, "/calendars/user/personal/event2.ics");
        assert_eq!(result.entries[1].etag, "\"etag-222\"");
        assert!(result.failed_uris.is_empty());
    }

    #[test]
    fn parse_propfind_calendars_extracts_can_edit_from_privileges() {
        // iCloud / Fastmail / SOGo emit current-user-privilege-set on
        // shared calendars. A read-only delegation block should mark the
        // calendar as can_edit=false; a writable one as can_edit=true.
        let read_only = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/shared-readonly/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Shared (read-only)</D:displayname>
        <D:current-user-privilege-set>
          <D:privilege><D:read/></D:privilege>
          <D:privilege><D:read-current-user-privilege-set/></D:privilege>
        </D:current-user-privilege-set>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let calendars = parse_propfind_calendars(read_only);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].can_edit, Some(false));

        let writable = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Personal</D:displayname>
        <D:current-user-privilege-set>
          <D:privilege><D:read/></D:privilege>
          <D:privilege><D:write-content/></D:privilege>
          <D:privilege><D:write/></D:privilege>
        </D:current-user-privilege-set>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let calendars = parse_propfind_calendars(writable);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].can_edit, Some(true));
    }

    #[test]
    fn parse_propfind_calendars_can_edit_none_when_block_absent() {
        // Servers that don't emit current-user-privilege-set leave can_edit
        // at None; sync layer interprets None as "editable" for back-compat.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/old-server/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Old Server</D:displayname>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].can_edit, None);
    }

    #[test]
    fn parse_propfind_calendars_empty_privilege_set_leaves_can_edit_none() {
        // Round 3 #25 regression guard. Some test mocks (and a handful of
        // real servers handling unknown ACL gracefully) emit a self-closed
        // `<current-user-privilege-set/>` with no children. The previous
        // shape interpreted that as "explicit empty -> read-only" and
        // flipped can_edit=Some(false), suppressing edit affordances even
        // though no privilege information was actually conveyed. Anchor
        // the signal on the first `<privilege>` ancestor instead, so an
        // empty set stays "unknown" (None -> sync layer assumes editable).
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/empty-priv/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Empty Priv</D:displayname>
        <D:current-user-privilege-set/>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 1);
        assert_eq!(
            calendars[0].can_edit, None,
            "empty privilege-set should leave can_edit=None (treated as editable), not Some(false)"
        );
    }

    #[test]
    fn parse_propfind_calendars_recognizes_open_close_calendar() {
        // Some servers emit `<C:calendar></C:calendar>` rather than the
        // self-closed `<C:calendar/>`. Both must mark the response as a
        // calendar resource.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/work/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection></D:collection><C:calendar></C:calendar></D:resourcetype>
        <D:displayname>Work</D:displayname>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].href, "/calendars/user/work/");
        assert_eq!(calendars[0].display_name.as_deref(), Some("Work"));
    }

    #[test]
    fn parse_propfind_calendars_ignores_nested_href() {
        // SOGo / Radicale return privilege descriptors alongside the calendar
        // prop block. A nested `<href>` inside `<privilege>` must not clobber
        // the calendar's own `<href>`.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/work/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Work</D:displayname>
        <D:current-user-privilege-set>
          <D:privilege><D:read/></D:privilege>
          <D:owner><D:href>/principals/user/</D:href></D:owner>
        </D:current-user-privilege-set>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].href, "/calendars/user/work/");
    }

    #[test]
    fn parse_multiget_report_handles_cdata() {
        // Servers wrap large iCalendar payloads in CDATA. Without the
        // Event::CData arm we'd silently drop the body.
        let xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<D:multistatus xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\">\n\
  <D:response>\n\
    <D:href>/calendars/user/personal/event1.ics</D:href>\n\
    <D:propstat>\n\
      <D:prop>\n\
        <D:getetag>\"etag-111\"</D:getetag>\n\
        <C:calendar-data><![CDATA[BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\nUID:cdata-test@example.com\nSUMMARY:CDATA Event\nEND:VEVENT\nEND:VCALENDAR]]></C:calendar-data>\n\
      </D:prop>\n\
    </D:propstat>\n\
  </D:response>\n\
</D:multistatus>";

        let results = parse_multiget_report(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "/calendars/user/personal/event1.ics");
        assert!(results[0].1.contains("CDATA Event"));
    }

    #[test]
    fn parse_propfind_events_skips_collection_with_etag() {
        // Davical / Bedework / old Zimbra emit a sub-collection without a
        // trailing slash and with a getetag - the third-arm fallback in
        // is_icalendar_resource would have admitted it as an event resource.
        // Inspecting <resourcetype><collection/></resourcetype> rejects the
        // entry instead, so the follow-up REPORT doesn't 403 on it.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/work-subcollection</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:getetag>"collection-etag-123"</D:getetag>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/work/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let result = parse_propfind_events(xml);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].uri, "/calendars/user/work/event1.ics");
        // The collection href has a 2xx propstat - it just isn't an event
        // resource. NOT a server-reported failure, so it must not show up
        // in failed_uris (otherwise sync would treat it as a sticky entry
        // forever).
        assert!(result.failed_uris.is_empty());
    }

    #[test]
    fn is_icalendar_resource_accepts_calendar_xml() {
        // RFC 6321 xCal: application/calendar+xml is a valid event form
        // alongside text/calendar. The previous shape silently skipped it.
        assert!(is_icalendar_resource(
            "/cal/event.xml",
            "application/calendar+xml; charset=utf-8"
        ));
    }

    #[test]
    fn is_icalendar_resource_handles_query_string_on_ics() {
        // /cal/event.ics?revision=42 ends with neither `.ics` nor `/`,
        // so the bare extension check missed it. Strip query/fragment
        // before the extension check.
        assert!(is_icalendar_resource(
            "/cal/event.ics?revision=42",
            ""
        ));
    }

    #[test]
    fn normalize_calendar_color_folds_apple_argb() {
        use super::normalize_calendar_color;
        assert_eq!(normalize_calendar_color("#0000FFFF"), "#0000FF");
        assert_eq!(normalize_calendar_color("#abcdef00"), "#abcdef");
    }

    #[test]
    fn normalize_calendar_color_passes_rgb_through() {
        use super::normalize_calendar_color;
        assert_eq!(normalize_calendar_color("#0000FF"), "#0000FF");
        assert_eq!(normalize_calendar_color("blue"), "blue");
    }

    #[test]
    fn parse_propfind_events_preserves_weak_etag() {
        // RFC 7232 weak ETag round-trip: the `W/"..."` form must survive
        // parsing untouched so it can be sent back verbatim in If-Match.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendars/user/personal/weak.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>W/"weak-etag-111"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let result = parse_propfind_events(xml);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].etag, "W/\"weak-etag-111\"");
        assert!(result.failed_uris.is_empty());
    }

    #[test]
    fn parse_ctag_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <CS:getctag>ctag-value-12345</CS:getctag>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let ctag = parse_ctag(xml);
        assert_eq!(ctag.as_deref(), Some("ctag-value-12345"));
    }

    #[test]
    fn parse_multiget_report_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <C:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:test-1@example.com
SUMMARY:Test Event
DTSTART:20240315T100000Z
DTEND:20240315T110000Z
END:VEVENT
END:VCALENDAR</C:calendar-data>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let results = parse_multiget_report(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "/calendars/user/personal/event1.ics");
        assert!(results[0].1.contains("Test Event"));
    }

    #[test]
    fn parse_propfind_calendars_skips_404_propstat_values() {
        // Mixed propstat block: one 200 OK with the resourcetype + display
        // name, one 404 Not Found with a stale ctag echoed by the server.
        // The previous parser merged values from both; the propstat-status
        // gate should now pull only from the 200 block and ignore the 404.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
               xmlns:CS="http://calendarserver.org/ns/">
  <D:response>
    <D:href>/calendars/user/personal/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/><C:calendar/></D:resourcetype>
        <D:displayname>Personal</D:displayname>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
    <D:propstat>
      <D:prop>
        <CS:getctag>stale-ctag-from-cache</CS:getctag>
      </D:prop>
      <D:status>HTTP/1.1 404 Not Found</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let calendars = parse_propfind_calendars(xml);
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].href, "/calendars/user/personal/");
        assert_eq!(calendars[0].display_name.as_deref(), Some("Personal"));
        // The 404 propstat's stale ctag must NOT leak through.
        assert!(calendars[0].ctag.is_none());
    }

    #[test]
    fn parse_propfind_events_skips_response_level_500() {
        // A 207 response can carry per-resource error responses with a
        // top-level <status> sibling of <propstat>. Those entries used to
        // be filtered out only when they had no etag, so an etag-bearing
        // 500 entry could leak through.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event2.ics</D:href>
    <D:status>HTTP/1.1 500 Internal Server Error</D:status>
  </D:response>
</D:multistatus>"#;

        let result = parse_propfind_events(xml);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].uri, "/calendars/user/personal/event1.ics");
        // Round 3 #51: the 500 entry must show up in failed_uris so the
        // sync layer preserves the local copy across this round rather
        // than mistaking a server-reported failure for "absent".
        assert_eq!(
            result.failed_uris,
            vec!["/calendars/user/personal/event2.ics".to_string()]
        );
    }

    #[test]
    fn parse_propfind_events_flags_propstat_404_as_failed() {
        // Round 3 #51: a 207-OK response whose only propstat is non-2xx
        // (e.g. a 404 from a transient server error or a permission flap)
        // must NOT be silently dropped - the sync layer would then treat
        // the href as absent and delete the local copy. Surface it via
        // failed_uris instead so the local copy is preserved.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/calendars/user/personal/event1.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <D:getcontenttype>text/calendar</D:getcontenttype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/event2.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag/>
      </D:prop>
      <D:status>HTTP/1.1 404 Not Found</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
        let result = parse_propfind_events(xml);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].uri, "/calendars/user/personal/event1.ics");
        assert_eq!(
            result.failed_uris,
            vec!["/calendars/user/personal/event2.ics".to_string()]
        );
    }

    #[test]
    fn parse_multiget_report_skips_response_level_500_with_stale_data() {
        // The SOGo failure shape: response-level 500 alongside a 200
        // propstat that echoes stale calendar-data from the cache. The
        // previous parser would emit the stale data; the response-status
        // gate now drops it.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/personal/good.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-111"</D:getetag>
        <C:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:good@example.com
SUMMARY:Good Event
DTSTART:20240315T100000Z
DTEND:20240315T110000Z
END:VEVENT
END:VCALENDAR</C:calendar-data>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/calendars/user/personal/broken.ics</D:href>
    <D:propstat>
      <D:prop>
        <D:getetag>"etag-stale"</D:getetag>
        <C:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:stale@example.com
SUMMARY:STALE DATA
DTSTART:20240314T100000Z
DTEND:20240314T110000Z
END:VEVENT
END:VCALENDAR</C:calendar-data>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
    <D:status>HTTP/1.1 500 Internal Server Error</D:status>
  </D:response>
</D:multistatus>"#;

        let results = parse_multiget_report(xml);
        assert_eq!(results.len(), 1, "stale 500 response must not be emitted");
        assert_eq!(results[0].0, "/calendars/user/personal/good.ics");
        assert!(results[0].1.contains("Good Event"));
        // The stale data from the 500-response was dropped, so it must not
        // appear anywhere in the results.
        for (_, ical) in &results {
            assert!(!ical.contains("STALE DATA"));
        }
    }

    #[test]
    fn parse_multiget_report_skips_per_resource_404() {
        // Per-resource 404 with no calendar-data (resource deleted between
        // PROPFIND and REPORT). Should be naturally dropped by the existing
        // emission gate (no calendar-data) AND by the new response-status
        // check, but the new path also catches the case where the server
        // emits the empty propstat block alongside the 404.
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response>
    <D:href>/calendars/user/personal/missing.ics</D:href>
    <D:status>HTTP/1.1 404 Not Found</D:status>
  </D:response>
</D:multistatus>"#;

        let results = parse_multiget_report(xml);
        assert!(results.is_empty());
    }
