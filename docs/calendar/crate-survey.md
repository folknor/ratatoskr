# Calendar Library Survey

Survey date: 2026-05-02

This survey ran after 4 rounds of review on the calendar code. The
trigger was: "we keep finding RRULE / CalDAV / TZ edge cases — are
there existing crates that would let us reduce surface?"

Question we set out to answer: are there existing Rust crates for iCalendar
parsing, vCard parsing, RRULE expansion, and TZID/Windows-zone resolution
that we should be using instead of (or in addition to) calcard and our
hand-rolled RRULE engine? If yes, which one. If no, what should we be
copying from the better-designed candidates.

The five Tier 1+2 source trees are checked out under `research/` for
follow-up reading. Per-crate deep-read reports were used to produce this
document; the durable findings have been folded in here.

## Where ratatoskr stands today

Snapshot of the calendar surface as of the survey date, so the rest of the
document has somewhere concrete to land. Verify line numbers if you're
acting on this — they will drift.

**calcard call sites (4):**

- `crates/core/src/caldav/parse/ical/mod.rs` (~690 lines) — iCalendar
  parsing. Uses `calcard::Parser`, `calcard::Entry`, `calcard::icalendar::*`,
  `calcard::common::PartialDateTime`. ~32 match sites against
  `ICalendarProperty::*` / `ICalendarValue::*` enums.
- `crates/core/src/carddav/parse.rs` (~504 lines) — vCard parsing for live
  CardDAV sync. Uses `calcard::vcard::{VCard, VCardProperty, VCardValue}`.
- `crates/import/src/vcard_parser.rs` (~237 lines) — vCard parsing for
  `.vcf` import. Same calcard surface as `carddav/parse.rs` —
  near-duplicate code that could collapse if we owned the parser.
- `crates/graph/src/calendar_sync.rs:658` (~30 lines) — Microsoft Windows
  TZ name → IANA, via `calcard::common::timezone::Tz::from_str`. Only
  consumes calcard's baked-in Windows alias table; everything else in
  this site is our code.

**Hand-rolled RRULE engine (1):**

- `crates/db/src/db/queries_extra/calendars/view/rrule/mod.rs` (1243
  lines) + `tests.rs` (928 lines). 4 rounds of review. Review-findings
  open items #9 (DST-gap zero-duration) and #16 (sparse YEARLY 40k-step
  walk) live here.
- A comment at `mod.rs:58-60` explains why we don't reuse calcard's
  resolver inside expansion. Worth deleting once a parser-side decision
  is made.

**TZID handling:**

- Parse-time TZID resolution uses calcard's `TzResolver` inside
  `caldav/parse/ical/mod.rs`. The parser stores the resolved IANA name on
  the event row as a string.
- RRULE expansion re-parses that string with `chrono_tz::Tz::from_str`
  rather than reusing a calcard resolver. Explicit choice — we don't pull
  calcard into the db crate.

**Open review-findings cross-reference** (in `docs/calendar/review-findings.md`):

- #9 — DST-gap collapses `wall_duration` to 0. Lives in our RRULE engine.
- #16 — `YEARLY_MAX_STEPS=80_000` cost note for sparse YEARLY rules.
- #18 — `time.rs::resolve_through_gap` 1-minute walk; decided to keep.
- #38, #39 — weak-ETag drop in `caldav/client.rs` (out of scope for this survey).
- #47 — empty `SUMMARY:` not distinguishable from absent. **Blocked on
  calcard upstream**, unblockable while we use calcard's parser.
- #45, #46, #50 — feature work, out of scope for this survey.

## Crates surveyed

| Crate | Scope | LOC | License | Verdict |
|---|---|---|---|---|
| calcard | iCal + vCard + JSCalendar + JSContact + RRULE + TZ | ~22k src | Apache-2.0 OR MIT | Tier 1 — current dep |
| caldata-rs | iCal + vCard + RRULE expansion + TZ | ~20k | Apache-2.0 | Tier 1 |
| dateutil-rs | python-dateutil port: parser, RRULE, tz, relativedelta | ~20k | MIT | Tier 1 (RRULE only) |
| calendar-crates (calico/rfc5545-types/jscalendar/calendar-types) | iCal types + parser, JSCalendar, no vCard, no expansion | ~32k | MIT | Tier 2 |
| eventix | High-level scheduling on top of `icalendar` + hand-rolled recurrence | ~11k | MIT/Apache-2.0 | Tier 2 |
| icalendar (hoodie) | iCal parser/builder | ~8.7k | MIT/Apache-2.0 | Tier 3 |
| truth-engine | RRULE/conflict/freebusy on top of `rrule` | ~11k | MIT/Apache-2.0 | Tier 3 |
| defernodate | RRULE expansion only (struct API, no parser) | ~1k | MIT | Tier 3 |
| ezcal | iCal + vCard parser/serializer | ~5k | MIT | Tier 3 |

Tiers reflect adoptability for ratatoskr's profile (high-volume Exchange /
CalDAV import, 5+ years of history, deep search, deduplication). Tier 1 is
viable today; Tier 2 has design ideas worth lifting but is not adoptable
whole; Tier 3 is informative-or-not, but not a candidate.

## Cross-cutting patterns

What separates the well-designed candidates from the rest is consistent and
worth naming.

### Streaming vs batch parsing

calcard, caldata-rs, and dateutil-rs all expose **streaming** parsers
(iterator over `Entry` / `Component`). One bad line is `Entry::InvalidLine`,
not a parse abort. icalendar and ezcal parse the whole input in one shot
(`many1(all_consuming(...))` or full-buffer split) — one bad line aborts the
whole calendar. For ingest from servers that emit non-spec output (Exchange,
some CalDAV implementations), batch parsing is a liability.

calcard goes one step further: errors are `Entry` variants, not a `Result`.
The caller decides whether to halt or skip. For a CalDAV importer ingesting
mixed-quality bodies that's the right shape.

*Ratatoskr today:* consumes calcard's streaming `Parser::next() → Entry`
in `caldav/parse/ical/mod.rs`. Already on the right shape.

### AST shape

Five distinct strategies showed up:

1. **Typed property name + typed value enum** (calcard, caldata's typed
   struct half, calico). `enum Property { Dtstart(DateOrTime), Summary(Text), ... }`
   plus `enum Value { Text(...), DateTime(...), ... }`. Best for compile-time
   exhaustiveness on consumers; worst for verbatim round-trip.
2. **Property bag with verbatim content lines** (caldata's `Vec<ContentLine>`
   half). Pairs well with strategy 1 — known fields are typed, unknowns
   round-trip without lossy interpretation. Costs allocations.
3. **Typed struct with hoisted fields + extras** (ezcal's
   `Event { dtstart, summary, ..., extra_properties: Vec<Property> }`). Loses
   parameter typing; opinionated on what's "common".
4. **Property-only with no enum** (icalendar's `BTreeMap<String, Property>`).
   Stringly-typed parameters, last-wins on duplicates, no compile-time
   exhaustiveness.
5. **No AST, takes pre-built structs** (defernodate, truth-engine). Skips
   the parsing problem entirely.

Strategy 1+2 hybrid (caldata's approach) is the strongest design for our
needs: typed access for consumers we own, lossless round-trip for the long
tail.

*Ratatoskr today:* we don't have our own AST — we walk calcard's typed
enums at ~32 match sites in iCal + ~16 in vCard. If we owned an AST, the
1+2 hybrid is what we'd build.

### Line unfolding

calcard, caldata-rs, and calico all do unfolding **inside the tokenizer**
with `Cow<[u8]>` / `Cow<str>` (borrowed-when-clean, owned-on-fold). icalendar
and ezcal allocate a fresh `String` for the entire input every parse. For
a 20k-event CalDAV initial sync, the difference is real.

All Tier-1 unfolders handle both CRLF and bare LF correctly — this matters
because lots of real-world iCal data is LF-only despite RFC 5545 mandating
CRLF.

*Ratatoskr today:* calcard's tokenizer-internal Cow unfolder is the path
we're on. Nothing to change.

### Escape handling

Two genuine bugs surfaced in **icalendar (hoodie)** specifically. From
`icalendar.md`:

- Multi-pass `replace()` chain in `parsed_string.rs:38` is ordering-dependent.
  `\\,` decodes to `,` — the literal-comma intent is silently lost.
- `\:` is decoded to `:` despite not being a valid RFC 5545 escape, and the
  writer doesn't re-escape it — round-trip is lossy on `\:` values.

calcard handles unescaping inside the tokenizer with explicit table dispatch.
caldata explicitly **does not unescape** in the parser (values stored raw)
— the caller unescapes on consumption. That's a defensible choice for
round-trip fidelity but pushes work onto every consumer site.

*Ratatoskr today:* we get pre-unescaped values from calcard. If we
switched to caldata, every consumer site (~48 across iCal + vCard
parsers) would need an explicit unescape pass. Real cost.

### Empty vs absent

Only **caldata-rs** distinguishes `SUMMARY:` (present, empty) from no
SUMMARY at all in its public API. calcard's parser drops empty values at
parse time (this is the upstream blocker on ratatoskr's review-findings
#47). icalendar and ezcal don't expose the distinction at all.

*Ratatoskr today:* blocked on calcard upstream. Switching to caldata
would unblock review-findings #47 by itself — the only candidate where
this is true.

### Duplicate properties

Three different policies:

- **caldata-rs**: strict — `PropertyConflict` error on dup `DTSTART`.
  Spec-correct, surfaces malformed input at parse time.
- **calcard**: collected into a `Vec`, with `property()` first-wins and
  `expand_dates` last-wins. Caller decides.
- **icalendar / ezcal**: silent last-wins via `BTreeMap::insert`. No
  diagnostic.

For an importer that wants to tag-and-ingest rather than reject, collected
+ caller-decides is the right shape.

*Ratatoskr today:* we wrote `pick_datetime_property` to deal with calcard's
non-deterministic iteration order on duplicates. With caldata's strict
mode we'd drop that helper but might lose ingest robustness on malformed
real-world input — tradeoff.

### TZID resolution

Three approaches, in descending sophistication:

- **calcard**: full `TzResolver` as a public type. Resolves inline VTIMEZONE,
  then X-LIC-LOCATION, then the Microsoft CDO TZID alias table (~700 entries),
  then IANA name fallback.
- **caldata-rs**: ~700-entry Microsoft TZID PHF table compiled in. No
  pluggable resolver trait, but covers the same ground for fixed inputs.
- **calico**: parses TZID, never resolves it.
- **icalendar / ezcal**: store TZID as a string, hand it to the consumer.

Anyone replacing calcard while keeping ratatoskr's Microsoft Exchange
support pulls in this Windows alias table somehow. The CLDR `windowsZones.xml`
is the upstream canonical source.

*Ratatoskr today:* parse-time uses calcard's `TzResolver` (via
`caldav/parse/ical/mod.rs`); RRULE expansion re-parses the resolved name
with `chrono_tz::Tz::from_str` (in `view/rrule/mod.rs`). The Graph crate
uses `Tz::from_str` directly to consume calcard's Windows alias table —
that's the call site explicitly tagged for replacement (see
`graph/src/calendar_sync.rs:658`).

### RRULE expansion

Four implementations across the candidates, all derived from `rust-rrule`
genealogically:

- **calcard's `datecalc/`** — fork of `rust-rrule` with SPDX preserved.
  Full FREQ/BYDAY-with-ordinals/BYSETPOS, EXDATE, RDATE periods,
  RECURRENCE-ID overrides with `RANGE=THISANDFUTURE` offset propagation.
  RFC 7529 (rscale, by_easter) parsed but not iterated.
- **caldata-rs** — also vendored from `rust-rrule`. Same feature set;
  RANGE=THISANDFUTURE handled inline by template-switching mid-iteration.
- **dateutil-rs** — independent re-implementation. Strongest engineering of
  the four: typed `Frequency`, `ByList<T> = SmallVec<[T;7]>`, bitmask
  BY*-filters, const-built yearday/monthday tables (~4.5 KB), batched buffered
  iterator with reusable wnomask/nwdaymask/eastermask buffers, sub-daily
  skip-ahead via `mod_distance`. RRuleSet uses `BinaryHeap` merge.
- **defernodate** — wraps `rrule 0.14` directly; calls `.all(u16::MAX)` (no
  laziness). Not viable.

DST handling sits in different layers per crate. dateutil-rs handles it in
its TZif parser, expansion engine works in `NaiveDateTime`. calcard handles
it inside the iterator using calcard's own `Tz` enum (`Floating` /
`Fixed` / `Tz`). Eventix has the cleverest DST gap *fallback*: a
`resolve_local` that picks the pre-gap offset and threads `intended_time`
through `skip_subdaily_to_matching_day` so wall-clock time doesn't drift.

*Ratatoskr today:* hand-rolled engine in `view/rrule/mod.rs` (1243 +
928 LOC). Post-4-rounds. Open items #9 (DST-gap zero-duration) and #16
(sparse YEARLY 40k-step walk) live here. None of the survey candidates
solve #9 cleanly except eventix's pre-gap-offset pattern, which is a
~50-line port, not a dep swap.

### RECURRENCE-ID overrides

calcard and caldata-rs both **replace** the master occurrence (correct).
calico / eventix / icalendar emit the override **alongside** the master
(visibly wrong — the user sees a duplicate at the original time). Open
calcard issue #14 (in their tracker, see "calcard open issues" in the
calcard verdict below) is about this same bug being still present in a
corner case despite the engine generally getting it right.

*Ratatoskr today:* our `expand_recurrence_with_overrides` in the RRULE
engine handles RECURRENCE-ID by collecting overrides into a HashSet
keyed on canonical recurrence-id form, then skipping master instances
that match. RANGE=THISANDFUTURE is not handled — that's a known gap.

### Performance instincts

Wins observed:

- **`Cow`-preserving line readers** (calcard, caldata-rs, calico). Borrowed
  by default, owned only on fold or escape.
- **`phf::Map` for TZID alias tables** (caldata-rs). Compile-time perfect-hash,
  zero allocation per lookup.
- **Const yearday/monthday tables + bitmask BY-filters** (dateutil-rs).
- **`SmallVec<[T;7]>` for BY-lists** (dateutil-rs). Avoids heap allocation
  for the common short-list case.
- **Sub-daily skip-ahead via `mod_distance`** (dateutil-rs, eventix). Avoids
  per-step iteration for the dominant common case.

Concerns observed:

- **icalendar**: `fold_line` is O(n²) on long property values via repeated
  `chars().nth(...)`. Owned AST does ~3× the allocations of calcard.
- **ezcal**: per-line `to_uppercase()` allocations.
- **defernodate**: `.all(u16::MAX)` materializes 65535 occurrences eagerly,
  silent truncate above that. `get_instance` brute-forces `MIN_UTC..MAX_UTC`.
- **caldata-rs**: per-line `to_uppercase()` allocations, `lazy_static`
  instead of `OnceLock`.

### Code-quality signals

| Signal | calcard | caldata | dateutil | calico | eventix |
|---|---|---|---|---|---|
| `forbid(unsafe_code)` | yes | yes | one justified `unsafe` | one transmute | yes |
| Fuzz targets | 3 (libfuzzer) | none visible | none visible | none visible | none visible |
| MIRI compatible | yes | yes | yes | unsure | yes |
| Test corpus | 850 files | snapshot tests + insta | RFC vectors + proptest | 262 spec files (40 categorised failures) | proptest + criterion |
| Strict lint config | yes | yes | yes | yes | yes (`unwrap_used` warn) |
| `unwrap()` in non-test code | a few | a few | rare | rare | none |

Everyone in Tier 1 is engineered to a similar standard. The ranking
between them is a question of fit, not quality.

## Per-candidate verdicts

### calcard — Tier 1 (current dep, baseline)

Substantially more correct than the open issue tracker suggested. Streaming
parser with `Cow`-tokenization, full RRULE expansion, public `TzResolver`
type, RECURRENCE-ID with `RANGE=THISANDFUTURE` offset propagation, fuzz
+ MIRI + 850-file corpus.

The remaining hygiene concern is upstream pacing (one author, last commit
2025-12-12 as of survey date). Open issues that affect us (in
`stalwartlabs/calcard`):

- **#19** (Apr 2026): `DTSTART;VALUE=DATE:20260101` panics on the public
  `to_rfc3339()` method. All-day events crash on a basic API call.
  We work around by going through `PartialDateTime` directly.
- **#14** (Oct 2025): RECURRENCE-ID override emitted alongside the master
  in some cases instead of replacing. Engine generally gets this right;
  the bug is a corner-case regression.
- **#2** (Jul 2025): EXDATE with multiple values not folded on output per
  RFC 5545. Output-side, doesn't bite us on parse.

These are not engine-design problems — they're maintenance pace problems.

### caldata-rs — Tier 1 (top alternative)

Apache-2.0, lineage from `peltoche/ical-rs` (the long-standing crate this
forks-and-modernizes), maintainer `lennart-k` runs the `rustical` CalDAV
server so the parser is exercised in real CalDAV server code. Strict
singleton enforcement, 700+ entry Microsoft TZID PHF, typestate stages,
byte-level zero-copy line reader, RANGE=THISANDFUTURE inline.

Gotchas: parser intentionally does not unescape — caller does. No
pluggable resolver trait. A few `unwrap()` panics on malformed input.
4-month-old, 2 stars, 9 versions — small but credible.

### dateutil-rs — Tier 1 (RRULE only)

Not an iCalendar library — only relevant as an RRULE engine donor. But its
expansion engine is technically the strongest of any candidate: const tables,
bitmask filters, batched iterator with reusable masks, sub-daily skip-ahead.
Real TZif parser. Strict-quality code with `thiserror` and proptest.

If we were starting from zero on RRULE expansion alone, this is the design
to target. As a drop-in dep it's awkward (sits at a different abstraction
layer), but as a porting source it's the best of the lot.

### calendar-crates (calico/rfc5545-types) — Tier 2

Strongly-typed AST, not property-bag. `FreqByRules` enforces FREQ↔BY
admissibility at the type level. BY-sets are `NonZero<u64>` bitsets. ~450
tests, externally-tracked spec corpus with `tracey` requirements (262
files: 222 pass, 40 categorised failures — the honest failure tracking is
unusual and good).

No vCard. No RRULE expansion (types only). Public `ParseError` discards
rich error info from the internal `CalendarParseError` (fixable).

Best ideas: typed parameter slots, type-level FREQ↔BY admissibility,
externally-versioned spec corpus.

### eventix — Tier 2

The only candidate that bundles parse + expand + tz, but it delegates
parsing to hoodie/icalendar (so inherits its bugs) and the recurrence engine
is hand-rolled and narrow (no BYMONTH, no BYSETPOS, no ordinal BYDAY, no
RDATE/RECURRENCE-ID).

DST handling is the standout: `resolve_local` picks pre-gap offset on a
forward-fall, threads `intended_time` through `skip_subdaily_to_matching_day`
so wall-clock time doesn't drift. Worth reading for the pattern.

Other negatives: lossy lowering on import (VTIMEZONE blocks ignored,
RECURRENCE-ID dropped, ATTENDEE parameters dropped, DATE vs DATE-TIME lost),
all error variants `String`-bodied, `eprintln!` in library code.

### icalendar (hoodie) — Tier 3

Two real bugs: lossy `\\,` decoding, lossy `\:` round-trip. `MULTIS` table
treats `X-PROP` / `IANA-PROP` as exact strings instead of RFC notation.
DTSTAMP/UID synthesised at serialise-time so two serializations of the
same Event differ (bad for content-hashing). No streaming parser. No
VTIMEZONE walk. No vCard.

Builder-side ergonomics are good. Parser side is not. Adoption signal is
strong (405k downloads, 184 stars) but the actual code quality doesn't
match.

### truth-engine — Tier 3

Not a parser. Wraps `rrule v0.14` and does conflict/freebusy/availability
math. Synthesises text RRULE fragments to feed upstream. Public
`DstPolicy` is dead code. Out of scope for our problem.

### defernodate — Tier 3

Pure expander, takes Rust structs not text. Materialises `.all(u16::MAX)`
eagerly (65535 cap, silent truncate). Re-parses RRULE on every call. No
DATE-only (only DATE-TIME). Override model has a clean HashMap design but
the rest is not production-ready.

### ezcal — Tier 3

Single-pass full-buffer parse, allocation-heavy, escape/unescape duplicated
3× across files, builder methods `.expect(...)` panic on bad input. Honest
about scope (no expansion, no DST). One person, one version. Not adoptable.

## Deferred work catalog

Status as of 2026-05-02: all four items below are **deferred, not
abandoned**. The intent is to take advantage of as much of this as
possible — the survey was the prep work, not the conclusion. Calendar
code is on ice for a while after 4 rounds of review; pick up from this
catalog when we come back.

### 1. Rip calcard from the Graph timezone helper (~1 day)

Smallest, most independent piece. Three tasks:

1. Vendor the CLDR `windowsZones.xml` into a `phf::Map<&str, &str>` in
   either the `calendar` crate or the `types` crate. ~700 entries.
   Generate at build time from the upstream XML. Caldata's PHF approach
   (`research/caldata-rs/.../guess_timezone.rs`) is the model.
2. Replace the call in `graph/src/calendar_sync.rs:658` with a small
   `resolve_windows_tz(name) -> Option<chrono_tz::Tz>` that consults the
   PHF map then falls back to `chrono_tz::Tz::from_str`.
3. Drop the `calcard` dep from `graph/Cargo.toml`.

No interaction with parsers or RRULE engine. Could be done first.

### 2. Lift design ideas from dateutil-rs into our RRULE engine (~1-2 days)

Our hand-rolled `expand_recurrence` is post-4-rounds; replacing it whole
with calcard's or caldata's expander wouldn't be net-positive. But
dateutil-rs has perf ideas worth porting:

- Const-built yearday/monthday tables in place of our per-call computation
- Bitmask BY*-filters in place of our linear filter passes
- `SmallVec<[T;7]>` for BY-lists
- Sub-daily skip-ahead via `mod_distance` (eventix has this too — closes
  the dense-secondly-with-BYDAY case)

Localized refactors against existing well-tested behavior. Source to
read: `research/dateutil-rs/src/rrule/`.

### 3. Port eventix's DST-gap pattern to close review-findings #9 (~half day)

Eventix's `resolve_local` picks the pre-gap offset on a spring-forward
fall and threads `intended_time` through subsequent operations so
wall-clock time doesn't drift. ~50-line port. The current behavior in
our engine collapses `wall_duration` to 0 when DTSTART lands inside a
DST gap; this fixes that without changing our engine's overall shape.

Source to read: `research/eventix/src/timezone.rs` (`resolve_local`) and
`research/eventix/src/recurrence.rs`
(`skip_subdaily_to_matching_day`).

### 4. Migration to caldata as the parser (~1 week, larger lift)

This is the option with the most leverage: replaces 1431 LOC of calcard
glue across 4 sites with a parser maintainer we can trust more (Apache-2.0,
peltoche+lennart-k, used by `rustical` CalDAV server). Unblocks
review-findings #47 (empty-vs-absent) by itself.

Concrete change list:

- `crates/core/src/caldav/parse/ical/mod.rs`: replace ~32 match sites
  against `ICalendarProperty::*` / `ICalendarValue::*` with caldata's
  equivalents. Add an explicit unescape pass at consumption (caldata
  stores raw values).
- `crates/core/src/carddav/parse.rs` + `crates/import/src/vcard_parser.rs`:
  same pattern for ~16 vCard match sites. Likely candidate for
  collapsing the duplication while we're in there.
- `crates/graph/src/calendar_sync.rs:658`: caldata's `PROPRIETARY_TZIDS`
  PHF can replace either calcard or our own vendored CLDR table from
  step 1. Decide which.
- `crates/db/src/db/queries_extra/calendars/view/rrule/mod.rs:58-60`:
  drop the comment about pulling calcard into the db crate (no longer
  relevant). The hand-rolled engine itself stays — caldata's expander is
  also forked rust-rrule, no upgrade.
- `Cargo.toml` (workspace): swap `calcard` for `caldata` across `core`,
  `graph`, `import`.
- `pick_datetime_property` in our parser becomes obsolete (caldata's
  strict singleton enforcement errors on dup DTSTART) — but consider
  whether ingest robustness on malformed real-world input matters more
  than spec-correctness here.

Risk: caldata is younger and smaller than calcard. The "fork-and-own"
escape hatch is realistic if it goes silent — Apache-2.0, ~20k LOC,
clean module structure.

### Things deliberately not on the catalog

- Adopting hoodie/icalendar — bugs documented in its verdict.
- Adopting the third-party `rrule` crate — year-stale, 30 open issues.
- Forking calcard ourselves — premature; caldata is a viable upstream.

## Ideas worth lifting (with attribution)

For ratatoskr's calendar code generally:

- **`TzResolver` as a first-class type** (calcard) — exposing TZID
  resolution as an API surface rather than always-implicit. Useful when
  ingesting events that reference VTIMEZONEs from a different VCALENDAR.
- **Components stored flat with `Vec<u32>` child IDs** (calcard) — avoids
  nested `Vec<Component>` cloning, gives stable identifiers usable as
  `comp_id` in expansion output, and keeps mutation cheap. Directly relevant
  for a DB-backed calendar where component IDs map to row IDs.
- **Strict singleton enforcement on parse** (caldata-rs) — surface
  malformed input at parse time rather than papering over later.
- **Const-generic typestate (`<const VERIFIED: bool>`)** (caldata-rs) — the
  same struct definition serves both partial-during-build and
  complete-after-build at the type level, without separate types for each
  stage. The technique generalizes well beyond calendars.
- **Const yearday/monthday tables + bitmask BY-filters** (dateutil-rs) —
  RRULE engine perf at no semantic cost.
- **`SmallVec<[T;7]>` for BY-lists** (dateutil-rs) — common-case heap-free.
- **Stack-buffer lowercasing for case-insensitive lookup** (dateutil-rs) —
  one justified `unsafe` block, but avoids the per-line `to_uppercase()`
  allocation that bloats most parsers.
- **DST gap-resolution with pre-gap offset + intended_time threading**
  (eventix) — closes our review-findings #9 cleanly.
- **Lazy occurrence pipeline via chained iterators** (eventix) —
  `take_while` + `filter` + `take` so excluded events don't consume the
  count budget. Better than pre-expanding to `Vec` and filtering.
- **Bounded-iteration guard with `_capped` opt-in** (eventix) — error when
  both COUNT and UNTIL are absent rather than looping until OOM, with an
  explicit "I know what I'm doing" entry point.
- **Strict, fail-loud RRULE parser** (eventix) — reject unsupported parts
  at parse time rather than silently degrading to a broader schedule than
  the user wrote.
- **Type-level FREQ↔BY admissibility (`FreqByRules`)** (calico) — catch
  malformed combinations at construction rather than expansion.
- **Cleanly factored crate split** (calico workspace) — shared primitives
  in `calendar-types`, iCalendar-specific in `rfc5545-types` + `calico`,
  JSCalendar parser-agnostic via `JsonValue` traits with `serde_json` behind
  an opt-in feature. Worth considering if our calendar code grows.
- **Four-shape `DatePerhapsTime` enum** (icalendar) — modeling DATE /
  floating DATE-TIME / UTC DATE-TIME / TZID DATE-TIME as four discriminated
  variants rather than sniff-the-string heuristics. The pattern is right
  even though the rest of the crate isn't worth adopting.
- **External spec corpus with categorised failures** (calico's
  `CALICO-CORPUS-ERRORS.md`) — honest about what works, what doesn't, what's
  intentionally unsupported. Better than our scattered review-findings notes.

## Status snapshot

| Area | Today | Catalog item | Notes |
|---|---|---|---|
| iCal parsing | calcard | #4 caldata migration | Deferred |
| vCard parsing | calcard (2 sites) | #4 caldata migration | Deferred; opportunity to collapse duplication |
| RRULE expansion | hand-rolled | #2 dateutil-rs perf port; #3 eventix DST-gap port | Deferred |
| Graph TZ resolution | calcard | #1 vendor CLDR + drop dep | Deferred but smallest piece |
| `view/rrule/mod.rs:58-60` comment | Stale | Drop with #4 | Trivial cleanup tied to migration |

For how the deferred catalog interacts with the open punch list in
`docs/calendar/review-findings.md`, see that document's *Sequencing relative
to `crate-survey.md`* section. Net: nothing in the punch list blocks the
catalog; three items are entangled with specific catalog items and should
be folded in when those are picked up; four items are independent.

## When to reassess

Triggers that should change the urgency or shape of the catalog:

- **calcard ships a fix for #14, #19, or #2** — urgency drops a notch on
  catalog #4. We're less exposed.
- **calcard goes 6+ months without a commit** (last as of survey:
  2025-12-12) — caldata migration urgency rises. Run `gh api repos/stalwartlabs/calcard/commits` to check.
- **calcard 1.0 release with breaking API** — reassess #4. Migration cost
  may equalize between "follow calcard upstream" and "switch to caldata".
- **caldata maintainer goes silent** (no commits 4+ months) — fork
  conversation reopens. Apache-2.0, ~20k LOC, viable. Status: check
  `gh api repos/lennart-k/caldata-rs/commits`.
- **Review-findings #9 (DST-gap zero-duration) bites a real user** —
  promote catalog #3 to top.
- **Review-findings #47 (empty-vs-absent) bites a real user** — promote
  catalog #4 to top; only caldata unblocks it.
- **Build a second consumer of the iCalendar AST** (e.g., calendar export,
  CalDAV server side) — catalog #4 leverage grows; the current 1431 LOC
  of glue would have to be duplicated against a different surface.

The fork-and-own conversation from the jmap-client precedent: not yet. Two
upstream crates have to fail before we should fork — calcard would have to
go silent or break interface, and caldata would have to fail to be a
viable migration target. As of survey date both are alive and engineered
well. Reassess at the trigger points above, not on a calendar.
