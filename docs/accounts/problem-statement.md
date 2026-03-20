# Accounts: Problem Statement

## Overview

Account setup is the first thing every user does. It must be fast, obvious, and require no technical knowledge. The user types their email address, authenticates, and their inbox appears. No provider selection, no server configuration, no jargon.

The backend infrastructure is complete — auto-discovery (registry, autoconfig XML, MX lookup, JMAP well-known, port probing), OAuth with PKCE for all major providers, and IMAP credential handling are all implemented. This document specifies the UI layer.

## First Launch

On first launch with no configured accounts, the app shows a centered modal over an empty window. The modal is generously sized — larger than the content strictly requires, giving the first interaction a spacious, unhurried feel. The app icon is displayed prominently above the title.

```
┌──────────────────────────────────────────────────┐
│                                                  │
│                    [App Icon]                     │
│                                                  │
│              Welcome to Ratatoskr                │
│                                                  │
│     Enter your email address to get started      │
│                                                  │
│     [alice@corp.com                           ]  │
│                                                  │
│     [Continue]                                   │
│                                                  │
│                                                  │
└──────────────────────────────────────────────────┘
```

This is the same "Add Account" modal used elsewhere (see § Add Account Flow). It can be dismissed, but nothing in the app will work without at least one account.

## Add Account Flow

### Step 1: Email Address

The modal shows a single email input field and a Continue button. Enter also submits.

When the user submits, discovery runs. A spinner or progress indicator replaces the Continue button while discovery is in progress. Discovery has a 15-second timeout.

### Step 2: Discovery Result

Discovery returns a ranked list of protocol options (Gmail API, Microsoft Graph, JMAP, IMAP). The best match is auto-selected.

**Happy path (single clear result):** The user never sees the discovery result. The flow proceeds directly to authentication.

**Multiple options:** A card list appears showing the available protocols, ranked by confidence. The top option is highlighted. Each card shows:

- Provider logo/icon
- Provider name (e.g., "Gmail", "Microsoft 365", "IMAP")
- Server details for IMAP (e.g., "imap.corp.com:993")

The user selects one and continues.

**Discovery failed:** The modal shows "We couldn't auto-detect your mail server" and falls back to a manual configuration form. The user first selects a provider type from a card list with logos (Gmail, Microsoft 365, JMAP, IMAP), then fills in the provider-specific fields:

- For IMAP: incoming server, port, security (SSL/TLS/STARTTLS), outgoing server, port, security
- For JMAP: session URL
- Auth method (OAuth / Password)

This manual form is the escape hatch. It should be functional but is not the primary path.

### Step 3: Authentication

**OAuth providers (Gmail, Microsoft, JMAP with OAuth):**

The system browser opens to the provider's OAuth consent page. The app shows a waiting state:

```
┌──────────────────────────────────────────┐
│                                          │
│  Complete sign-in in your browser        │
│                                          │
│  Waiting for authorization...            │
│                                          │
│  [Cancel]                                │
│                                          │
└──────────────────────────────────────────┘
```

OAuth completes via a local redirect (localhost:17248). On success, the flow proceeds to Step 4. On failure or cancel, an error is shown with a Retry button.

**Password auth (IMAP):**

The modal shows the full server configuration alongside credentials. Fields are pre-filled from discovery where available. IMAP and SMTP credentials are configured together but with the option to use separate SMTP credentials.

```
┌──────────────────────────────────────────────────┐
│                                                  │
│  Incoming (IMAP)                                 │
│  Server: [imap.corp.com    ] Port: [993 ]        │
│  Security: [SSL/TLS ▾]                           │
│  Username: [alice@corp.com                     ] │
│  Password: [••••••••                           ] │
│                                                  │
│  Outgoing (SMTP)                                 │
│  Server: [smtp.corp.com    ] Port: [587 ]        │
│  Security: [STARTTLS ▾]                          │
│  ☐ Use different credentials for SMTP            │
│  Username: [alice@corp.com                     ] │
│  Password: [••••••••                           ] │
│                                                  │
│  ☐ Accept self-signed certificates               │
│                                                  │
│  [Sign In]                                       │
│                                                  │
└──────────────────────────────────────────────────┘
```

Password fields display plaintext — no masking with dots or asterisks.

The SMTP username/password fields are hidden by default and shown when "Use different credentials for SMTP" is checked. When unchecked, SMTP uses the same credentials as IMAP.

The "Accept self-signed certificates" checkbox is for corporate environments with internal CAs. It defaults to unchecked.

Some providers require app-specific passwords (e.g., Gmail with 2FA but no OAuth, Yahoo). When discovery detects this, a help link is shown below the password field pointing to the provider's app-password setup page (these URLs are in the discovery registry).

### Step 4: Success

The account is created, tokens are encrypted and stored, and sync begins. The modal closes. For first launch, the user sees their inbox populating. For subsequent account additions, the new account appears in the account list.

No confirmation screen, no "setup complete" page. The inbox appearing is the confirmation. Sync progress is shown in the status bar (see `docs/status-bar/problem-statement.md`).

## Account Management

Account management lives in Settings. It shows a list of configured accounts, each as a card.

### Account Card

```
┌──────────────────────────────────────────────────────┐
│ 🔵 alice@corp.com                                    │
│ Microsoft 365 · Last synced: 2 minutes ago           │
└──────────────────────────────────────────────────────┘
```

- **Color indicator** — the account's assigned color (used in contact pills, calendar indicators, etc.)
- **Email address** — prominent
- **Provider name** — e.g., "Gmail", "Microsoft 365", "IMAP (imap.corp.com)"
- **Last sync time** — relative timestamp

### Account Actions

Clicking an account card slides in an editor (same pattern as contact management). Available actions:

- **Account name** — editable, a user-chosen label for the account (e.g., "Work", "Personal"). Used in the account dropdown, contact pills, From selector in compose, and anywhere accounts are visually distinguished. Auto-generated from the email domain on account creation (e.g., "corp.com" → "Corp").
- **Display name** — editable, used as the sender name in outgoing email
- **Account color** — editable, used in contact pills, calendar indicators, and anywhere accounts are visually distinguished. Assigned automatically from the label color palette on account creation, but user-configurable.
- **Re-authenticate** — triggers the OAuth flow again (for expired/revoked tokens) or password re-entry for IMAP
- **CalDAV settings** — for IMAP/JMAP accounts that use CalDAV for calendar. Shows CalDAV URL, username, and lets the user configure the connection if auto-discovery didn't find it.
- **Delete account** — removes the account and all its data (labels, threads, messages, attachments, cached files). Prompts for confirmation with a clear warning about data deletion. The deletion cascades through all related tables.

### Adding Another Account

An "Add Account" button at the bottom of the account list opens the same Add Account modal (§ Add Account Flow).

### Account Selector

The main app sidebar has an account dropdown at the top. It shows:

- **All Accounts** — unified view across all accounts (default)
- Individual accounts listed below

Selecting an account scopes the sidebar navigation (folders, labels) and thread list to that account. The calendar view always shows all accounts (calendar visibility is controlled by the calendar list toggles, not the account selector).

Keyboard shortcut for cycling accounts is already implemented.

## Error States

### Token Expiry

When an OAuth token expires and refresh fails, the status bar (see `docs/status-bar/problem-statement.md`) shows a persistent warning for the affected account. Clicking it opens the re-authentication flow. The account's data remains visible (cached locally) but sync is paused until re-authentication.

### Connection Failure

Transient network errors are retried silently with backoff. Persistent failures (wrong server, certificate errors) are shown in the status bar (see `docs/status-bar/problem-statement.md`) with the error message.

### Duplicate Account

Attempting to add an account with an email address that's already configured shows an error: "This account is already configured." No duplicate accounts are allowed.

## Open Questions

1. ~~**Account colors**~~ **Resolved.** Assigned automatically from the label color palette in order of account creation. User-configurable in the account editor.

2. ~~**Account reordering**~~ **Resolved.** Users can reorder accounts in the settings account list (drag to reorder). The order is reflected in the sidebar account dropdown.

3. ~~**Default send-from account**~~ **Resolved.** See `docs/sidebar/problem-statement.md` § Default sender account for compose. Resolution order: explicit selection → thread context → last manually selected sender → current scope → first account.
