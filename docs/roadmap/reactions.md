# Reactions

**Tier**: 2 — Keeps users from going back
**Status**: ❌ **Not implemented**

---

- **What**: Emoji reactions on email messages (Exchange/new Outlook feature)

## Cross-provider behavior

| Provider | Native support |
|---|---|
| Exchange (Graph) | Full — `reactions` collection on message |
| Gmail API | Nothing |
| JMAP | Nothing |
| IMAP | Nothing |

## Pain points

- Phase 1 priority: even before displaying reactions, must not break when a message has reaction metadata. Defensive deserialization — ignore unknown fields rather than erroring.
- Display: reactions appear as a row of emoji chips below the message (like Slack/Teams). Each chip shows the emoji + count + who reacted. This is a new UI element with no existing equivalent in the client.
- Local-only reactions for non-Exchange: could implement local-only reactions that only the user sees. Questionable value — reactions are social, local-only defeats the purpose. Probably better to just not show the reaction UI on non-Exchange accounts.
- Sync: reactions can change after initial sync (someone reacts later). Need to handle updates to the reactions collection during delta sync.
- Compose: adding a reaction is a PATCH to the message on Graph. Need to handle the case where the user reacts to a message but is offline (queue and sync later? or require connectivity?).

## Work

Phase 1 — defensive deserialization. Phase 2 — display reactions on Exchange messages. Phase 3 — allow reacting on Exchange accounts. Skip local fallback.
