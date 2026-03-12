# @Mentions

**Tier**: 2 — Keeps users from going back
**Status**: ❌ **Not implemented**

---

- **What**: `@User` in email body, recipient gets the message auto-flagged
- **Dependency**: Contacts & Groups sync (Tier 1)

## Cross-provider behavior

| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `mentions` collection on message | Sync mention metadata, auto-flag mentioned user's copy |
| Gmail API | Nothing | Local-only: detect @-patterns in body, no server-side flagging |
| JMAP | Nothing | Local-only |
| IMAP | Nothing | Local-only |

## Pain points

- Display: Exchange stores mentions as structured metadata separate from the body HTML. The body contains the display text ("@John Smith") but the `mentions` collection has the resolved email/user ID. Need to correlate the two for highlighting.
- Compose: need @-autocomplete that triggers on `@` character in the compose editor, searches unified contacts, and inserts both the display text and the mention metadata (for Exchange accounts).
- Non-Exchange accounts: can still insert "@John Smith" text in the body (it's just text), but there's no server-side flagging. The recipient's client won't auto-flag it. Acceptable degradation — the visual cue in the body is still useful.
- Parsing incoming @mentions from non-Exchange senders: some people manually type "@Name" in emails. No metadata to parse — could attempt heuristic matching against contacts, but likely not worth the false positives.

## Work

Display mentions on Exchange messages, @-autocomplete in compose using unified contacts, insert mention metadata for Exchange sends, text-only fallback for other providers.
