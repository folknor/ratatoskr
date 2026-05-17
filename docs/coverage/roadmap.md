# Coverage: Implementation Roadmap

Phased plan for the coverage system described in `docs/coverage/problem-statement.md`. Each slice is independently shippable and resolves a specific subset of the Open Questions left in the problem statement.

The roadmap follows the staged delivery the problem statement commits to: build the mechanism, pilot it on one bounded area, harden the loader for that area, generalize, then promote verification to compile time. Backfill is treated as work, not as a separate phase that happens "after" the system is built. The mechanism is incomplete until it has been used.

## Slice 1: Marker Scheme and Doc Parser

The foundational decision. The marker scheme is what makes a doc section a registered contract. Every later slice depends on the parser knowing how to find and extract IDs.

**What needs to be built:**

- A marker scheme that is visible in rendered markdown without breaking reading flow, carries a stable ID, survives doc reorganization, and is greppable from a tool.
- A parser that walks `docs/` and returns the set of `(file, section, contract_id)` triples plus any associated metadata (status, enforcement hint).
- A lint that rejects duplicate IDs, malformed markers, invalid metadata, orphaned markers (a marker with no surrounding section), and misplaced markers.
- A smoke catalog: mark up three to five sections in `reference/architecture.md` to validate the scheme before committing to it. Examples: action service as mutation gate, generation counters for async safety, folder-vs-label semantics.

**Design decisions to resolve in this slice:**

- Marker form. Decision: HTML comment immediately below the section header (`<!-- coverage: architecture.action_service_as_mutation_gate -->`). Blank lines are allowed between heading and marker; prose or other content before the marker is rejected. The marker renders invisibly, is easy to grep, and does not require a markdown extension.
- ID grammar. Decision: lowercase dotted slug, with each segment starting with a letter and continuing with lowercase letters, digits, or underscores. The dotted prefix is conventional rather than mechanically tied to the file path.
- How a contract that spans sibling sections is represented. Likely one marker on the parent section, with the children implied. Alternatively, multiple markers sharing an ID.
- Status field. Decision: `required` by default, plus `gap`, `retired`, and `compile-enforced`.
- Enforcement hint. Decision: small enum, currently `compiler`, `rust-test`, `lua-harness`, `convention`, and `mixed`.

**Current implementation notes:**

- The initial parser lives in `crates/coverage/` and exposes a `ratatoskr-coverage` process entrypoint. This keeps Ratatoskr independent of brokkr's source while leaving a concrete command brokkr can wrap later.
- The smoke catalog currently marks five sections in `reference/architecture.md`: action service gate, provider trait abstraction, generation counters, folder-vs-label semantics, and adding a new email action.
- The parser ignores fenced code blocks so docs can show marker examples without registering duplicate contracts.

**Out of scope for this slice:**

- Enforcing Lua frontmatter in the harness loader.
- Project-wide coverage failure.
- Marking up every architectural boundary in the doc. The smoke catalog is enough to validate the scheme.

**Depends on:** Nothing. This is the foundation.

## Slice 2: Lua Claim Format

The other foundational decision. The Lua harness loader already parses frontmatter for execution metadata; this slice adds a required `covers` field.

**What needs to be built:**

- A frontmatter format for `covers` in Lua harness scripts that lists one or more contract IDs.
- A read-only parser that scans `crates/app/tests/sync-harness/` and `crates/app/tests/service-harness/` for claims.
- A later update to the existing harness loader to parse and reject malformed pilot-area claims when strict mode is enabled.
- Validation that IDs are syntactically valid (matches the grammar from slice 1). Existence-check against the registry is reported by the read-only tool.
- A short author-facing note in the harness doc (`reference/glossary/harness.md`) describing the format.

**Design decisions to resolve in this slice:**

- Syntactic form. Decision: repeated `-- @covers: id` lines in the initial frontmatter comment block. Each line names exactly one contract ID. Comma-separated or space-separated multi-ID lines are rejected.
- Whether a missing `covers` is a hard error from the start or a warning until slice 5. Decision for now: missing claims are reported, not loader errors, until a pilot area is backfilled and strict mode is scoped to that area.

**Out of scope for this slice:**

- Failing on unknown claimed IDs.
- Rust-side claim mechanism (slice 6).

**Depends on:** Slice 1 (uses the ID space, but parser does not need to validate against registry yet).

## Slice 3: `brokkr coverage` v1 (Read-Only Reporting)

The first user-visible deliverable. A subcommand that reads doc anchors and Lua frontmatter, computes the diff, and reports it. Reporting only, no enforcement, no build failures.

**What needs to be built:**

- A `brokkr coverage` subcommand (name to be finalized in this slice).
- Integration with the slice 1 parser to load the registered contract set.
- Integration with the slice 2 frontmatter parser to load the claim set.
- A diff producing four lists:
  - Registered contracts with zero Lua claims.
  - Lua tests with no `covers` claim.
  - Lua tests claiming an ID that is not in the registry.
  - Lua tests with claims that are syntactically malformed.
- Plain-text output by default. JSON output via a flag for later consumption.
- Exit code zero regardless of findings. Reporting only.

**Current implementation notes:**

- `ratatoskr-coverage report [WORKSPACE_ROOT]` already produces the read-only plain-text report shape from the Ratatoskr side. The brokkr integration remains pending because brokkr lives in a separate repository.
- Contracts with `status=gap`, `status=retired`, `status=compile-enforced`, or `enforcement=compiler` remain visible in the report but are excluded from the "registered contracts with no Lua claim" list.

**Design decisions to resolve in this slice:**

- Subcommand name. `brokkr coverage` is the working title; `brokkr cov` is shorter.
- Output sort order. By contract ID, by file, by area.
- Whether to surface compile-enforced contracts (slice 1 enforcement hint) separately or fold them into the same report. Folding is simpler; separating is honest.

**Out of scope for this slice:**

- Rust claim mechanism (slice 6).
- Failing the build (slice 5 for pilot, slice 7 project-wide).
- Codegen (slice 8).

**Depends on:** Slice 1, Slice 2.

## Slice 4: Pilot Area Backfill

The first real use of the system. Pick one bounded area, bring its contracts to exhaustive registration and its tests to exhaustive claim. This is where the model is validated and adjusted.

**What needs to be built:**

- Pick a pilot area. The leading candidate is provider sync coverage, because the architectural contracts are concentrated in a small set of doc sections and the existing Lua sync harness already covers many of them. An alternative pilot is the email action recipe, which is one procedural contract with multi-site enforcement and a small footprint.
- Mark every behavioral contract in the pilot area's docs with the slice 1 marker scheme. This means walking the relevant sections of `reference/architecture.md` and any related subsystem docs and adding markers.
- Add `covers` claims to every Lua harness script in the pilot area.
- Run `brokkr coverage` and triage each remaining gap as one of: write the missing test now, register as known gap with a tracking issue, or retire the contract.
- Document the pilot outcome in this roadmap (what worked, what the marker scheme had to absorb, what the Lua format had to absorb).

**Design decisions to resolve in this slice:**

- Pilot area choice.
- How a "known gap" is expressed. Likely a status on the contract marker (`status: gap`, `tracking: issue-url`) or a separate `gaps.toml` file. The former keeps it local to the doc; the latter keeps the doc clean.

**Current implementation notes:**

- Pilot area chosen: folders-labels semantics. The pilot contracts live in `reference/glossary/folders-labels.md`, backed by focused sync-harness claims for Graph categories as tag labels, JMAP mailboxes as container folders, account-scoped Gmail label identity, canonical system IDs, and provider-prefixed non-system IDs.
- `ratatoskr-coverage report --area glossary.folders_labels` is the narrow report loop for this pilot. `ratatoskr-coverage report --area architecture` shows the architecture-level smoke catalog.
- `brokkr check -p coverage` now enforces the pilot report shape through a repository smoke test: `glossary.folders_labels` must have no uncovered contracts and no unknown Lua claims.

**Out of scope for this slice:**

- Backfill of areas outside the pilot.
- Hardening the loader. The pilot validates that the model can reach exhaustive coverage; slice 5 makes that exhaustion enforced.

**Depends on:** Slice 3 (the tool surfaces the gaps that drive the backfill).

## Slice 5: Loader Hardening for the Pilot Area

Flip the switch on the pilot. The harness loader rejects pilot-area tests that fail validation. `brokkr coverage` exits non-zero when the pilot area has uncovered contracts. The rest of the tree remains advisory.

**What needs to be built:**

- Lua harness loader rejects a pilot-area test that has no `covers` claim.
- Lua harness loader rejects a pilot-area test that claims an unknown ID.
- `brokkr coverage --strict --area=<pilot>` (or equivalent scoping) exits non-zero when the pilot area has registered contracts with zero claims.
- Integration with `brokkr check`: the strict invocation is wired into the default check flow so violations break CI for the pilot area.

**Current implementation notes:**

- The Ratatoskr-side equivalent exists as `ratatoskr-coverage report --area glossary.folders_labels --strict`.
- The dedicated brokkr command and harness-loader rejection are still pending. Until then, the coverage crate smoke test gives the pilot an in-workspace regression gate.

**Design decisions to resolve in this slice:**

- Scoping mechanism. How the tool knows which contracts and tests are in the pilot area. Likely a path prefix on doc paths and test paths.
- Error message format. Loud failures must be actionable.

**Out of scope for this slice:**

- Project-wide hardening (slice 7).

**Depends on:** Slice 4 (pilot must be at exhaustive coverage before enforcement is meaningful).

## Slice 6: Rust Claim Mechanism

Rust integration tests need to claim contracts on the same footing as Lua harness scripts. The crate-boundary lockdown tests under `crates/service-state/tests/` are the canonical example. This slice settles how a Rust test makes a claim.

**What needs to be built:**

- A claim mechanism for Rust integration tests. Candidates: an attribute macro (`#[covers("architecture.action_service_as_mutation_gate")]`) that expands to a registration hook, a doc-comment marker (`/// @covers: ...`) parsed by an external tool, or a per-crate sidecar manifest (`tests/coverage.toml`). The attribute macro is the most ergonomic but requires a proc-macro crate; the doc-comment marker is the lightest weight and matches the Lua approach.
- Parser/extractor for the chosen mechanism.
- Update to `brokkr coverage` to read Rust claims alongside Lua claims.
- Apply the mechanism to the lockdown tests and any other Rust integration tests that back contracts (the boundary sections in `architecture.md` that name a Rust test will need attention).
- Distinguish compile-enforced contracts in the report: a contract with `enforcement: compiler` does not require a Rust or Lua claim, but should still be visible.

**Design decisions to resolve in this slice:**

- Mechanism choice (proc-macro, doc-comment, sidecar).
- Whether Rust tests and Lua tests share an ID namespace (yes, by the problem statement) or are validated separately.
- How a contract enforced purely by typed IDs or `#[must_use]` is represented. Likely a contract status of `enforcement: compiler` set in the doc marker, with no claim expected.

**Out of scope for this slice:**

- Codegen (slice 8). The Rust claim mechanism in this slice is parsed externally by the tool; promotion to a compile-time enum lives in slice 8.

**Depends on:** Slice 1 (ID space). Practically lands after Slice 5 so the pilot has stabilized the model before adding the Rust dimension.

## Slice 7: Full Backfill and Project-Wide Hardening

Repeat the pilot pattern across the rest of the tree. Mark every architectural boundary, every numbered Core Requirement in subsystem problem-statements, every procedural recipe. Add claims to every Lua harness script and every Rust integration test that backs a contract. Flip strict mode on project-wide.

This is the largest slice by volume. It is likely subdivided into commits per doc area:

- `reference/architecture.md` boundaries.
- `docs/command-palette/` Core Requirements and Decisions.
- `reference/glossary/folders-labels.md` semantics.
- `reference/glossary/overlay-surfaces.md` invariants.
- `reference/glossary/harness.md` (harness contracts, including the action recipe).
- Any other subsystem problem-statements that surface during the work.

**What needs to be built:**

- Per-area: mark contracts, add claims, triage gaps, then enable strict mode for that area.
- A final pass that removes the area-scoped flag from `brokkr coverage` and turns strict mode into the default.
- Documentation update: a short addition to `AGENTS.md` or the harness glossary telling new test authors that `covers` is required.

**Design decisions to resolve in this slice:**

- Order of areas. Areas with stable docs first (architecture boundaries), areas with churning docs later (command palette is still pre-V1).
- Whether retired or pre-V1 contracts get a different status that excludes them from strict mode while keeping them visible.

**Out of scope for this slice:**

- Codegen (slice 8).

**Depends on:** Slice 5, Slice 6.

## Slice 8: Codegen and Compile-Time Enforcement

Promote the tool checks to compile time. This is the end-state described in the problem statement: a Rust compiler error when a contract has no claim or a claim references a missing contract.

**What needs to be built:**

- A new `coverage` crate in the workspace whose `build.rs` discovers doc anchors and Lua harness scripts (and Rust integration tests if the slice 6 mechanism is doc-comment or sidecar based).
- Generated Rust artifacts:
  - `enum ContractId { ... }` with one variant per registered contract.
  - `const LUA_TESTS: &[LuaTest]` where each entry carries `covers: &[ContractId]`.
  - `const RUST_TESTS: &[RustTest]` (if applicable; the proc-macro variant of slice 6 would populate this differently).
- Compile-time assertions. Either `const _: () = assert!(...)` patterns over the generated tables, or build-script panics. Failures: a variant with zero claims; a claim with no matching variant; a duplicate variant.
- `brokkr coverage` retained as a developer-facing reporting tool that wraps the same data. The codegen and the tool share the parser.

**Design decisions to resolve in this slice:**

- Codegen trigger. A `build.rs` that re-runs on doc or harness change, or a `brokkr` step that writes a checked-in file. The former is honest about staleness but slows clean builds; the latter is fast but creates a generated file that can drift.
- How the build script discovers Lua scripts. Cargo's rerun-if-changed surface, or a manifest.
- How the assertions are expressed. `const_assert!` is the most direct; a build-script panic is the most general.
- Where the `ContractId` enum is consumed. Probably re-exported from a few high-traffic crates so test authors can refer to it ergonomically.

**Depends on:** All prior slices. The catalog must be stable and the claim mechanisms settled before they are promoted to compile-time artifacts.

## Dependency Graph

```
Slice 1 (markers + parser)
  +-> Slice 2 (Lua frontmatter)
  +-> Slice 3 (brokkr coverage v1)
        +-> Slice 4 (pilot backfill)
              +-> Slice 5 (loader hardening for pilot)
                    +-> Slice 6 (Rust claim mechanism)
                          +-> Slice 7 (full backfill + project-wide hardening)
                                +-> Slice 8 (codegen, compile-time enforcement)
```

Slice 1 and Slice 2 can land in parallel. Slice 6 can begin earlier in principle but lands after Slice 5 so the model is stable. Slice 8 is the only slice that strictly requires every prior slice.

## Risks and Mitigations

- **Marker scheme regret.** If the scheme picked in slice 1 turns out to be wrong, the pilot in slice 4 is the place to catch it. The marker is mechanical to rewrite while the catalog is small. The cost of regret rises sharply after slice 7.
- **Pilot area mis-selection.** A pilot too small does not validate the model; a pilot too large makes slice 4 expensive. The provider sync candidate is sized for one to two weeks of backfill work; the email action recipe candidate is one to three days. Sizing favors the smaller pilot if doubt exists.
- **Rust mechanism debate.** Slice 6's mechanism choice is the most contested decision in the roadmap. The mitigation is that slice 7 backfill cannot start until slice 6 settles, so the pressure to decide is real but the decision can be made in isolation against a stable Lua-only baseline.
- **Codegen build-time cost.** Slice 8's `build.rs` scans docs and Lua on every change. The mitigation is that Cargo's rerun-if-changed surface is narrow enough to keep the cost bounded, and the tool stage already proves the parser is fast enough for an interactive command.

## Success Criteria Per Slice

- Slice 1: A doc lint passes on the smoke catalog. The parser returns the expected triples for those sections.
- Slice 2: The harness loader parses `covers` fields without breaking existing tests. A test author can add a claim and see it surface in slice 3 output.
- Slice 3: `brokkr coverage` produces a useful report on the smoke catalog. Unknown claim IDs and missing claims are visible.
- Slice 4: The pilot area has exhaustive contract registration and every pilot test has at least one claim. `brokkr coverage` reports zero gaps for the pilot area (or every gap is a tracked, registered gap).
- Slice 5: An author cannot add a malformed pilot-area test. `brokkr check` fails when pilot-area contracts regress.
- Slice 6: Rust integration tests can claim contracts. The lockdown tests carry claims. Compile-enforced contracts are visible in the report.
- Slice 7: Every architectural boundary, every Core Requirement, and every procedural recipe in the docs is a registered contract. Every Lua harness script and every relevant Rust integration test claims at least one. Strict mode is the default.
- Slice 8: A contract added to the docs without a claim breaks the Rust build. A claim referencing a missing contract breaks the Rust build. The tool stage continues to work as a reporting interface.
