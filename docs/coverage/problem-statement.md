# Coverage: Problem Statement

## Overview

Ratatoskr's Lua harness has become a real test suite. Between the
Service subprocess harness, provider sync harness, and Lua-backed mock
fixtures, coverage now spans boot, dispatch, crash recovery, sync,
actions, calendar, contacts, attachments, search extraction, provider
writeback, and multi-account behavior.

That scale creates a new problem: it is no longer enough to ask "do we
have a test for this?" by searching filenames or reading `TODO.md`.
The real question is:

**Which coverage do we require, which coverage do we have, and which
coverage is missing?**

This is a coverage matrix problem. Each test covers one or more
specific behavior points across dimensions like protocol, resource,
operation, topology, auth mode, persistence target, and failure mode.
Those dimensions need to become queryable metadata instead of implicit
knowledge in test names.

## Current State

### What exists

- Lua Service harness scripts under `crates/app/tests/service-harness/`.
- Lua sync harness scripts under `crates/app/tests/sync-harness/`.
- Lua and TOML sync fixtures under `crates/app/tests/sync-fixtures/`.
- Brokkr discovery of harness scripts via frontmatter.
- Per-script frontmatter for basic execution metadata such as
  description, expected result, fixture, protocol, and ceiling.
- Request logs, DB state queries, fixture snapshots, and summary output
  that make the scripts useful as behavioral tests.

### What is missing

- No canonical list of required coverage points.
- No stable coverage IDs that tests can claim.
- No tool that answers "which required coverage is uncovered?"
- No tool that flags tests claiming unknown or retired coverage.
- No clear distinction between required, deferred, blocked, expected
  failure, and optional exploratory coverage.
- No queryable matrix for protocol/resource/operation/topology cells.
- No way to see fixture coverage, unused fixtures, or missing fixture
  schema smoke coverage.
- No durable way to prevent duplicate tests from accumulating around
  the same behavior while adjacent behavior stays untested.

## The Problem

### Test files are not the coverage model

Filenames are useful for humans, but they are not a coverage registry.

For example, a script named
`jmap-mailbox-secondary-create-import.lua` communicates intent, but it
does not answer:

- Which required behavior does this satisfy?
- Is this one test enough for the requirement, or is it one case in a
  larger matrix?
- Does it cover account scoping, DB label persistence, request-log
  routing, or all three?
- Is the missing negative case intentional?
- Is the corresponding primary-account case required?

The coverage model needs to live above individual test files.

### TODO prose is not a registry

`TODO.md` is good for work planning. It is not a reliable source of
truth for coverage.

As the harness grows, TODO prose becomes too coarse. One item can
unlock five scripts; one script can satisfy two requirements; one
blocked provider feature can hold open ten matrix cells. The tooling
needs structured requirements and structured test claims.

### Frontmatter is execution metadata, not coverage metadata yet

Current frontmatter tells brokkr how to run a script. It should also
tell coverage tooling what the script proves.

That does not mean every assertion goes in frontmatter. The script body
still owns exact behavior. Frontmatter should record stable, searchable
coverage facts:

- The script's identity.
- The fixture and protocol surface.
- The behavioral requirement IDs it covers.
- The matrix axes relevant to discovery and reporting.
- Whether the test is normal, expected-fail, blocked, flaky, expensive,
  or manual-only.

## Goal

Build a coverage registry and frontmatter contract that can answer:

- Which required coverage points have no test?
- Which tests claim coverage IDs that do not exist?
- Which coverage IDs are required but blocked by external systems?
- Which tests are expected to fail, and why?
- Which protocol/resource/operation/topology cells are empty?
- Which fixtures are unused?
- Which fixtures have no schema-load smoke coverage?
- Which tests overlap heavily with existing coverage?
- Which coverage is fast enough for default checks, and which belongs
  in longer cohorts?

The primary output is not a prettier test list. The primary output is a
missing-coverage report.

## Non-Goals

- Replacing test assertions with metadata.
- Making the registry a list of test files.
- Forcing every test into a rigid one-test-per-requirement shape.
- Building a general code-coverage tool.
- Measuring Rust line coverage.
- Measuring UI pixel coverage.
- Solving CI scheduling by itself, though the same metadata should help
  cohort selection later.

## Design Direction

### Registry of required coverage

The registry should define coverage requirements independent of test
files. A requirement is a stable behavior point that Ratatoskr expects
to keep covered.

Example shape:

```toml
[[requirement]]
id = "jmap.mailbox_set.secondary_create_imports_folder"
area = "sync"
protocol = "jmap"
resource = "mailbox"
operation = "remote_create"
topology = "multi_account"
asserts = ["account_scoping", "db_label_persistence"]
status = "required"
```

The exact file format is an implementation decision. TOML is a good
fit because the repo already uses TOML for fixtures and configuration.
The important part is that each requirement has a stable ID and a
structured set of axes.

### Tests claim coverage

Lua frontmatter should claim requirement IDs from the registry.

Example:

```lua
-- id: jmap-mailbox-secondary-create-import
-- area: sync
-- protocol: jmap
-- resource: mailbox
-- operation: remote_create
-- topology: multi_account
-- fixture: multi-account-secondary-primary.toml
-- covers:
--   - jmap.mailbox_set.secondary_create_imports_folder
-- expected: pass
-- ceiling: 120s
```

The test body remains the authority for the exact assertions. The
coverage claim says this script is intended to satisfy that registry
requirement.

### Tooling diffs registry against tests

The core tool should:

1. Load the registry.
2. Discover Lua tests and parse frontmatter.
3. Validate frontmatter shape.
4. Validate that every claimed `covers` ID exists.
5. Report every `status = "required"` requirement with no covering
   test.
6. Report every covered requirement whose tests are all expected-fail,
   blocked, skipped, or manual-only.
7. Emit matrix summaries by area, protocol, resource, operation, and
   topology.

The tool should be strict enough to catch drift but not so strict that
adding a useful test requires designing the entire matrix first.

## Matrix Axes

The initial registry should use a small set of axes that match how the
project already thinks about coverage.

### Area

High-level subsystem:

- `service`
- `sync`
- `actions`
- `calendar`
- `contacts`
- `attachments`
- `extract`
- `auth`
- `search`
- `settings`

### Protocol

External or internal surface:

- `service`
- `jmap`
- `graph`
- `gmail`
- `gcal`
- `caldav`
- `imap`
- `smtp`
- `local`

`local` is for behavior that does not cross a provider protocol.

### Resource

Object under test:

- `message`
- `thread`
- `mailbox`
- `label`
- `attachment`
- `calendar`
- `event`
- `contact`
- `contact_group`
- `signature`
- `token`
- `action`
- `index`
- `body_store`
- `inline_image_store`

### Operation

Behavior being exercised:

- `boot`
- `initial_sync`
- `delta_sync`
- `remote_create`
- `remote_update`
- `remote_delete`
- `local_create`
- `local_update`
- `local_delete`
- `writeback`
- `retry`
- `recovery`
- `failure`
- `shutdown`
- `crash_replay`
- `schema_load`

### Topology

Account and ownership shape:

- `single_account`
- `multi_account`
- `shared_mailbox`
- `public_folder`
- `delegated`
- `cross_protocol`
- `offline`

### Auth

Authentication shape when relevant:

- `none`
- `password`
- `oauth`
- `token_bound`
- `revoked`
- `expired`
- `reauth`

### Assertions

Important proof categories:

- `db_persistence`
- `request_log`
- `account_scoping`
- `shared_mailbox_scoping`
- `no_leakage`
- `search_index`
- `body_store`
- `inline_image_store`
- `attachment_cache`
- `pending_ops`
- `retry_state`
- `notification`
- `generation`
- `crash_safety`
- `provider_error`

The registry can add axes as needed, but adding axes should be
deliberate. A sparse, comprehensible matrix is better than a complete
taxonomy nobody maintains.

## Requirement Status

Requirements need status. Suggested initial values:

- `required` - must have at least one passing automated test.
- `covered` - optional alias for `required` once tooling can infer
  covered state. Prefer deriving this rather than storing it.
- `deferred` - desired coverage, but intentionally not required yet.
- `blocked` - desired coverage blocked by an external dependency,
  missing mock support, or missing product surface.
- `expected_fail` - known product bug or gap; test may exist and fail
  until fixed.
- `manual` - cannot be automated yet; document why.
- `retired` - old requirement retained only so stale test frontmatter
  can produce a useful warning.

The registry should include a short `reason` for any status other than
`required`.

## Frontmatter Contract

The minimum frontmatter fields for coverage-aware tests should be:

- `id` - stable test ID, normally matching the filename without `.lua`.
- `description` - human-readable summary.
- `expected` - `pass`, `fail`, or a named expected-failure state.
- `fixture` - fixture file if the script depends on one.
- `protocol` - primary protocol surface.
- `area` - high-level subsystem.
- `resource` - primary resource under test.
- `operation` - primary operation under test.
- `topology` - ownership shape.
- `covers` - list of registry requirement IDs.
- `ceiling` - existing timeout/backstop.

Optional fields:

- `auth`
- `asserts`
- `tags`
- `cohort`
- `cost`
- `blocked_by`
- `issue`
- `notes`

Frontmatter should remain readable in plain Lua comments. If structured
lists become awkward, the parser should support a small YAML-like
subset only for frontmatter, not arbitrary YAML features.

## Fixture Coverage

Fixtures need coverage too. The tooling should answer:

- Which fixtures are referenced by no test?
- Which TOML fixtures have no schema-load smoke test?
- Which Lua fixtures are executable fixtures rather than tests?
- Which tests depend on fixtures that are missing from the repo?
- Which fixture uses a protocol/resource shape that has no registry
  requirement?

This matters because saehrimnir fixture schema changes can break many
provider tests before Ratatoskr code is even involved. Schema-load
smoke coverage should catch stale fixture shapes early.

## Reporting

The first useful report should be text-first and suitable for local
developer use:

```text
coverage requirements: 184
required: 141
covered: 126
missing: 15
blocked: 22
expected-fail: 6

missing required coverage:
  graph.shared_mailbox.mail_sync.messages_scoped
  gmail.oauth_token_binding.labels_scoped
  imap.oauthbearer.account_binding.fetch_scoped

tests with unknown coverage IDs:
  jmap-old-mailbox-test.lua -> jmap.mailbox.old_id

fixtures without schema smoke:
  graph-categories-small.toml
```

Later reporting can include matrix tables, JSON output for CI, and
HTML summaries, but the first tool should optimize for answering the
developer's local question quickly.

## CI and Cohorts

Coverage metadata should eventually help CI choose what to run, but
coverage accounting and scheduling are separate concerns.

Useful cohort metadata:

- `smoke`
- `provider`
- `service`
- `crash`
- `writeback`
- `slow`
- `flaky`
- `manual`
- `destructive`

The missing-coverage report should not require running tests. It only
needs the registry and frontmatter. Runtime pass/fail status can be
layered on later from brokkr results.

## Open Questions

- Where should the registry live? Candidate:
  `docs/coverage/requirements.toml`.
- Should requirement IDs be globally flat strings, or hierarchical
  sections plus local IDs?
- Should `brokkr service-list` and `brokkr sync-smoke` parse the new
  coverage fields, or should a separate `brokkr coverage` command own
  this?
- Should coverage validation fail `brokkr check`, or only report
  warnings until the registry stabilizes?
- How strict should duplicate coverage warnings be?
- Should expected-fail tests be runnable by default, or isolated in a
  separate cohort?
- How should non-Lua Rust tests claim coverage requirements, if at all?

## Success Criteria

The coverage system is working when a developer can ask:

> Which tests are we missing?

and get a precise answer without reading every Lua file or manually
cross-checking TODO lists.

Concretely:

- Every required coverage point has a stable ID.
- Lua tests can claim one or more coverage IDs.
- Unknown coverage claims are reported.
- Missing required coverage is reported.
- Blocked and deferred requirements are visible but do not fail normal
  coverage validation.
- Fixture schema smoke gaps are visible.
- The report is useful before tests run.
- The metadata stays small enough that adding a test remains cheap.
