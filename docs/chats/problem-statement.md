# Chats: Problem Statement

## Overview

A significant portion of enterprise email is conversational - short, rapid,
informal exchanges between two people that read like chat messages but render as
full emails with headers, signatures, legal disclaimers, and quoted reply
chains. The friction between the conversational intent and the formal
presentation is a major reason people switch to Teams, Slack, or other chat
tools for quick 1:1 communication - then lose the archival, searchability, and
universal reachability that email provides.

Chats is a UI mode - not a protocol or a sync change. The underlying messages
are standard emails, sent and received through the user's existing accounts.
What changes is how they are presented. When a contact is designated as a "chat
contact," all direct 1:1 correspondence with that person renders as a chat
timeline instead of a traditional threaded email view.

This gives users the immediacy and lightness of chat without abandoning email's
strengths: it works across organizations, it's archival, it's searchable, and
it doesn't require the other party to be on any particular platform. The
recipient sees a normal email. Only the Ratatoskr user sees the chat view.

## The Problem

### Enterprise email is already conversational

Look at how people actually email their close colleagues:

```
Subject: Re: Re: Re: Re: quick question
Body: "yeah that works"
--
John Smith
Senior Director of Engineering
Acme Corp
123 Main St, Suite 400
...
[12 lines of legal disclaimer]
```

This is a chat message wearing a suit. The useful content is four words. The
rest is ceremony: a subject line that stopped being meaningful five replies ago,
a signature block the recipient has seen a thousand times, a legal disclaimer
neither party reads, and a quoted reply chain that duplicates the entire
conversation history in every message.

### The switching cost is real

When people move these conversations to Teams or Slack, they gain conversational
UI but lose:

- **Universal reachability** - the other person needs to be on the same
  platform
- **Archival** - chat history retention policies are often shorter than email;
  messages may be deleted, edited, or lost when someone leaves the organization
- **Search** - email search is mature, local, and fast; chat search is often
  cloud-dependent and limited
- **Formality gradient** - in chat tools, everything feels informal; there's no
  way to shift register when the conversation turns serious
- **Cross-organization communication** - email works with anyone who has an
  email address

### Email clients don't distinguish conversational exchange from formal email

Every major email client renders 1:1 exchanges the same way it renders mailing
list threads, mass notifications, and formal correspondence. There is no
recognition that a rapid back-and-forth between two people is fundamentally
different from a group discussion or a newsletter.

## What Chats Is

### Explicit opt-in per contact

Chats is not automatic. The user explicitly designates specific contacts as
"chat contacts." This is a deliberate choice - typically for close colleagues,
direct reports, or frequent collaborators where the communication pattern is
already conversational.

This avoids the problem of trying to automatically detect "chatty" threads
(which would be fragile and surprising) and gives the user full control over
which conversations get the lightweight treatment.

The contact system is already built - provider sync (Graph, Google People API,
CardDAV), auto-collected seen addresses, contact groups, photo caching, and
FTS5 search are all implemented (see `docs/contacts/problem-statement.md`).
Chat contact designation layers on top of this existing infrastructure as a
boolean flag or separate table, not a new contact concept.

### A view mode, not a message type

The underlying emails are unchanged. Chat view is purely a rendering decision:

- Messages render as **bubbles** in a timeline - sent messages on one side,
  received on the other
- **Signatures are stripped** from the display (the full message remains
  accessible)
- **Subject lines are de-emphasized or hidden** - in a chat context, the
  subject is noise
- **Quoted reply chains are collapsed** - the timeline already shows the history
- **Newest messages are at the bottom**, scrolled to the latest
- **Attachments render inline** where possible (images, PDFs) rather than as a
  list

The user can always toggle back to traditional email view for any conversation.
No information is lost.

### Sidebar presence

The sidebar has four sections (see `docs/sidebar/problem-statement.md`):

1. Pinned searches
2. Universal folders (All Accounts) / Provider folders (single account)
3. Smart folders
4. Labels

Chats adds a **new section** between pinned searches (section 1) and
universal folders (section 2), making five sections total. The existing
section numbering used across other docs (sections 2-4) is unaffected -
Chats is inserted above them.

```
[📅] [Scope Dropdown    ]
[  ] [   Compose        ]

 from:alice ha..  ✕       ← Pinned searches
 2 hours ago

CHATS                      ← Chats (new)
├ Alice Smith       2m
│ yeah that works
├ Bob Jones        1h
│ sent the deck
└ Carol Chen       3d
  see you monday

Inbox              12      ← Universal folders (section 2)
Starred
...
```

Each chat contact entry shows:

- The contact's name (and avatar when available from the contact system's
  photo cache - during implementation, investigate whether the 50MB cache
  cap is configurable and sane for users with many chat contacts)
- A preview of the latest message (signature-stripped)
- Unread state indicated by bold text (consistent with the thread list)
- Relative timestamp of the last message

Ordered by the order in which the user designated them as chat contacts
(newest designation at the bottom), and user-reorderable via drag-and-drop.
This avoids the sidebar constantly reshuffling based on message activity,
which would be distracting in a section that shares space with stable
navigation items like folders and smart folders.

The Chats section is **not affected by scope**. Like pinned searches, smart
folders, and labels, it is always the same regardless of which account is
selected. A chat contact may correspond across multiple accounts - the chat
view aggregates this naturally (though threads themselves are always
single-account, per Ratatoskr's threading model).

The Chats section should be **collapsible** like the other sidebar sections,
and **absent entirely** when the user has no chat contacts designated (no empty
section cluttering the sidebar).

### Conversation grouping

Ratatoskr's threading engine (JWZ algorithm in `crates/sync/`) already groups
messages into threads. Threads are always single-account - a thread belongs to
one account.

For chat view, the interesting question is how to group threads into a single
chat timeline for a contact. A user may have dozens of threads with the same
person over months, each with a different subject line. Chat view should
present these as **one continuous conversation** (or at least a recent-first
stream), not as separate thread items.

This means the chat timeline is not a thread view - it's a **per-contact
message stream**, pulling messages from all threads between the user and that
contact. Thread boundaries become less important; chronological ordering of
individual messages becomes primary.

The query is essentially: "all messages where (sender = contact OR recipient =
contact), ordered by date."

A thread is either a chat or it isn't. If every message
in a thread has exactly two participants (the user and the chat contact),
it's a chat thread and appears in the chat timeline. If any message in the
thread has a third party (CC, additional To), the entire thread is not a
chat - it appears in the thread list as normal email. No partial inclusion,
no fork points, no automagic.

### What about multi-account contacts?

A user might email the same person from different accounts (work and personal).
The contact system handles deduplication by email address - if Alice has
alice@work.com and alice@personal.com, and the user has synced contacts or
seen-address records linking both addresses, they are the same contact.

In chat view, messages from all accounts appear in the same timeline, ordered
chronologically. Each message bubble can show a subtle account indicator (the
account's color dot or abbreviation) so the user knows which account context
they're in. This is the same approach used elsewhere in Ratatoskr's
multi-account model.

When composing a reply from the chat view, the default account is determined by
the most recent thread context (which account received/sent the last message),
consistent with the compose account resolution order defined in
`docs/sidebar/problem-statement.md` § Default sender account.

### Compose in chat mode

When the user is in a chat view and wants to reply, the compose experience
should reflect the chat context:

- A **simple text input at the bottom** of the timeline, not a full compose
  window
- No subject line field. Replies reuse the existing thread's subject. If
  there is no prior email history with the contact at all, the client
  generates a subject: `"Hello, {contact_first_name}"` (i18n-aware,
  localized string). If the user has configured an LLM, it generates the
  subject from the message body instead.
- Normal signature insertion - the recipient sees a standard email, so the
  user's configured signature is appended as usual (see
  `docs/signatures/implementation-spec.md`). The signature is hidden in
  the sender's own chat view via the same stripping logic.
- Enter to send, Shift+Enter for newline by default. A setting inverts
  this (Shift+Enter to send, Enter for newline) for users who prefer it.
  The input box expands upward as the user adds lines (overlaying the
  chat timeline above it, not pushing it down), up to a reasonable
  maximum before scrolling internally.
- Emoji shortcode translation (`:thumbsup:` → 👍, `:)` → 😊) inline as the
  user types, consistent with Teams/Slack/Discord behavior. (This should
  eventually be available in the regular compose editor too.)
- Drag-and-drop or paste for attachments/images


## Signature Stripping

Signature stripping is the critical technical challenge. In chat mode, repeated
signature blocks destroy the conversational feel. The approach needs to be
reliable for the specific contacts the user has designated, not universally
perfect.

### Why per-contact scoping helps

Because chat contacts are explicitly designated, the system has a significant
advantage: it will accumulate many messages from the same sender. If the last 20
messages from a contact all end with the same 8-line block, that block is almost
certainly a signature. This is a much easier problem than general-purpose
signature detection.

### Stripping strategy

A layered approach, from most to least confident:

1. **HTML client markers** (~100% confidence when present) - major email
   clients tag their own signatures with identifiable HTML structures.
   Since we already parse HTML in `common`, these are free:

   | Client | Signature marker |
   |---|---|
   | Gmail | `<div class="gmail_signature">` |
   | Gmail (quotes) | `<div class="gmail_quote">`, `<div class="gmail_extra">` |
   | Yahoo | `<div class="yahoo_quoted">` |
   | Outlook | `<hr id="stopSpelling">` |
   | Thunderbird | `<div class="moz-cite-prefix">`, `<blockquote type="cite">` |
   | Apple Mail | `<blockquote type="cite">` |

2. **Standard delimiter** - `-- \n` (RFC 3676). 100% reliable when present,
   but rare in practice - only Thunderbird/Mutt-era clients insert it.

3. **User's own signatures** - the user's configured signatures (stored in
   the `signatures` table) can be matched against their own sent messages
   with 100% confidence. See "Relationship to the signatures subsystem"
   below.

4. **Per-sender learned pattern** - extract the common trailing block across
   multiple messages from the same sender. This is novel to our chat contact
   use case - no existing library does this because they operate on single
   messages without sender history. High confidence after a few messages.

5. **Heuristic patterns** - valediction phrases ("Best regards",
   "Sincerely", etc.), "Sent from my iPhone" boilerplate, lines of
   dashes/underscores as separators. Well-understood, used by the
   `email_reply_parser` family of libraries (GitHub, Zapier, et al.).
   Moderate confidence - language-dependent and not universal.

6. **Quote removal** - strip `On <date>, <person> wrote:` quoted blocks
   and `>` prefixed lines. The chat timeline already provides the
   conversation context, so quoted reply chains are pure noise.

### Prior art

The open-source landscape for signature stripping is limited:

- **mailgun/talon** (Python) - the most ambitious attempt. Offers both
  heuristic and ML (SVM classifier) modes. Claims 90% accuracy;
  independently measured at ~25-30%. Unmaintained since 2016.
- **github/email_reply_parser** (Ruby, with ports to Python, PHP, and
  **Rust**) - pure regex/heuristic. Good for quote detection, basic for
  signatures. The Rust port (`email_reply_parser` crate, v0.1.2) exists
  but is minimal.
- **Carvalho & Cohen (CMU, 2004)** - foundational academic work. Line-level
  classification using features like email/URL/phone patterns, sender name
  presence, punctuation ratio, and line position. Achieved 99.37% accuracy
  with windowed sequence models. The feature set is relevant if we ever
  want an ML layer.

No existing library combines HTML marker detection with per-sender learning.
Our layered approach - HTML markers first, then per-sender patterns, then
heuristics - should significantly outperform any single technique.

### Graceful degradation

When confidence is low (new contact, unusual message format):

- **Collapse, don't delete** - show the message body clean, but provide a
  subtle "show full message" affordance. Zero information loss.
- **Learn over time** - confidence improves as more messages from the contact
  are processed.
- **Never strip aggressively on the first message** from a new chat contact.
  Wait until a pattern is established.

### Relationship to the signatures subsystem

Ratatoskr already has a signatures system for compose (`docs/signatures/`).
Signature stripping for chat view is a **separate concern** - the compose
signatures system manages the user's own outgoing signatures, while chat
stripping removes incoming signatures from display. These are different
codepaths with different data sources and different reliability requirements.
The user's own signatures (stored in the `signatures` table) serve as a
high-confidence stripping source for sent messages (layer 3 above).

## Scope and Constraints

### What Chats is NOT

- **Not a chat protocol** - no XMPP, no Matrix, no proprietary messaging. These
  are emails.
- **Not presence/typing indicators** - there is no real-time channel. The other
  person is using regular email. (Though if both parties use Ratatoskr, presence
  could be explored as a future extension.)
- **Not group chat** - this is 1:1 only. Group email dynamics are different
  enough that forcing them into a chat view would be awkward. (Could be explored
  later.)

### What makes this feasible

- **No server changes** - purely a client-side rendering decision
- **No protocol extensions** - standard IMAP/JMAP/Graph, standard MIME messages
- **No cooperation required** - the recipient doesn't need to do anything
  different
- **Threading engine already exists** - JWZ threading in `crates/sync/` already
  groups messages into threads
- **Body store exists** - message content is already parsed and stored in
  `bodies.db` (compressed) via `BodyStoreState`
- **Contact system exists** - contact sync, deduplication, seen addresses, photo
  cache, and FTS5 search are all implemented
- **Threads are single-account** - no cross-account thread complexity; the
  multi-account aspect is handled at the contact level, not the thread level

### What makes this hard

- **Signature stripping reliability** - the core technical risk. Mitigated by
  per-contact learning and collapse-not-delete.
- **Conversation grouping across threads** - presenting multiple threads with
  the same contact as a single chat stream requires a query model that operates
  at the message level rather than the thread level, which is different from how
  the rest of the app works. A long-running chat contact could have thousands of
  messages across years of correspondence - the timeline will need pagination
  or virtual scrolling.
- **1:1 detection** - reliably determining that a thread has exactly two
  participants (the user and the contact) requires checking all message headers
  in the thread, not just the latest. If any message has a third party, the
  entire thread exits chat view.
- **Compose UX expectations** - users will expect chat-level responsiveness
  (Enter to send). But email is not instant, and sent messages go through SMTP.
  The latency gap between expectation and reality needs to be managed.

## Open Questions

1. ~~**Thread boundaries in chat view**~~ **Resolved.** Date separators
   between messages on different days, same as chat apps. When the subject
   line changes, the new subject is shown in subtle small text directly
   above the first bubble with that subject - not a full separator, just
   enough to signal a topic change.

2. ~~**New message from chat contact in inbox**~~ **Resolved.** A 1:1 thread
   with a chat contact is a chat, not an email. It does not appear in the
   Inbox thread list - it only appears in the Chats section.

3. ~~**Chat contact designation UX**~~ **Resolved.** Chat designation is a
   toggle in the contact management UI - it's an explicit contact-level
   setting, not a thread-level action.

4. ~~**Search integration**~~ **Resolved: not needed.** The Chats sidebar
   section is the discovery surface. A search operator adds complexity for
   no clear use case.

5. ~~**Notification behavior**~~ **Resolved: no special treatment.**
   Ratatoskr does not have notifications. If notifications are added later,
   chat contacts can be revisited then.

6. ~~**Chat threads and search/Inbox interaction**~~ **Resolved.** Chat
   threads are excluded from the Inbox thread list view only - they are
   still normal emails in the database. They appear in search results like
   any other thread. Opening a chat marks all messages read. Un-designating
   a chat contact returns all their threads to the normal thread list view
   with no data changes.

## Phases (Overview)

Detailed implementation specs will be written per phase. Rough ordering:

1. **Data model + chat contact designation** - schema for marking contacts as
   chat contacts, core queries for fetching chat timelines (per-contact message
   streams across threads)
2. **Chat timeline view** - the bubble-based rendering of 1:1 message streams,
   including basic signature stripping and quote removal
3. **Sidebar integration** - the Chats section in the sidebar (between pinned
   searches and universal folders) with contact list, previews, and unread
   counts
4. **Chat compose** - the lightweight inline compose experience, including
   emoji shortcode translation
5. **Signature stripping refinement** - per-sender learning, confidence scoring,
   and the collapse/expand UX
6. **Polish** - conversation grouping heuristics, mixed-mode handling, date
   separators, settings and preferences
