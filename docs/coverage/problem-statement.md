# Coverage: Problem Statement

## Overview

Ratatoskr needs a coverage system that answers one question precisely:

> Which behavioral contracts do we require, which do we have tests for, and which are missing?

The system is modeled on the command palette principle. The command palette problem statement states:

> Every action the user can perform must be a registered command with a unique identity. There is no way to create an action without it being part of the palette.

The coverage system applies the same shape to behavioral contracts and tests:

> Every behavioral contract in Ratatoskr must be a registered requirement with a stable identity, and every test must claim at least one registered requirement. There is no way to add a contract without registering it, and no way to add a test without claiming what it covers.

This is not currently true. It must become true. Ideally the code architecture enforces it, the way the `CommandId` enum forces every command to be handled in `update()`.

This document describes the contract registry, the claim mechanism, and the staged path from a tool-only check to architectural enforcement. It does not describe a parallel requirements catalog and it does not propose a coverage axis matrix.

## Why Now

The Lua harness has grown into a real test suite. Service subprocess harness, sync harness, action harness, calendar harness, contacts harness, and Lua-backed mock fixtures cover boot, dispatch, crash recovery, sync, actions, calendar, contacts, attachments, search extraction, provider writeback, and multi-account behavior. Rust integration tests cover crate boundaries and lockdown invariants. The architecture doc lists multiple boundary sections, each with an `Enforcement:` paragraph naming the test or compile mechanism that backs it. Subsystem problem-statements like `docs/command-palette/problem-statement.md` enumerate Core Requirements as numbered prose.

There is no link between any of these. The boundary sections name enforcement in prose. The harness scripts have execution metadata in frontmatter but no claim of what behavior they prove. The numbered Core Requirements have no IDs at all. "Adding a new email action" is a 9-site procedural recipe with no checklist that the recipe was followed.

It is no longer enough to search filenames or read `TODO.md` to answer "do we have a test for this." The contracts already exist in the docs. The tests already exist in the tree. The missing piece is a stable identity for each contract and a structured claim from each test.

## The Principle

The catalog must be exhaustive, not aspirational. Every registered contract is something the codebase asserts is true. The missing-coverage report is derived: it lists registered contracts with zero claims.

The catalog must be enforced, not advisory. A test without a `covers` claim is malformed and the loader rejects it. A claim referencing an unknown ID is malformed and the loader rejects it. A registered contract with zero claims fails the build.

The catalog must be expensive to violate, not cheap. Adding a contract requires registering it. Removing a test requires either retiring its contract or finding another test that still claims it. The friction is the point. It is the same friction `CommandId` imposes today: adding a user-facing action is intentional work, because the compiler insists on it.

The catalog must live where engineers already look. Contracts are doc sections in `docs/architecture.md`, in subsystem problem-statements like `docs/command-palette/problem-statement.md`, and in procedural recipes like "Adding a New Email Action". The doc is the registry. There is no parallel TOML or YAML catalog.

## Current State

What exists today:

- Architectural boundary sections in `docs/architecture.md` with `Enforcement:` paragraphs naming compile-time mechanisms, Rust integration tests, or harness scripts in prose.
- Subsystem problem-statements with numbered Core Requirements (e.g. `docs/command-palette/problem-statement.md` sections 1 through 9).
- Procedural recipes scattered through the architecture doc (the email-action checklist is the canonical example).
- A growing Lua harness under `crates/app/tests/service-harness/` and `crates/app/tests/sync-harness/` with execution-metadata frontmatter (fixture, ceiling, expected, cohort).
- Rust integration tests under `crates/*/tests/` including the crate-boundary lockdown tests.

What does not exist:

- Stable IDs for any of the above contracts.
- A `covers` claim mechanism on Lua harness scripts.
- A `covers` claim mechanism on Rust integration tests.
- A tool that diffs registered contracts against claims.
- Any compile-time relationship between contracts and tests.

## Core Requirements

### 1. Stable Contract Identity

Every behavioral contract has a stable string ID. The ID format is a dotted slug rooted at the doc section it lives in.

Examples:

- `architecture.action_service_as_mutation_gate`
- `architecture.generation_counters_for_async_safety`
- `architecture.adding_a_new_email_action`
- `cmdk.requirements.exhaustive_command_registry`
- `cmdk.requirements.fuzzy_search_word_boundary_weighting`
- `cmdk.decisions.enum_for_command_ids`

The dots are convention for readability; the tool does not parse them. The ID is a key. Renames are explicit and tracked; the tool reports broken claims when an ID is renamed without updating its claimants.

### 2. Doc Sections Mark Themselves as Contracts

A doc section becomes a registered contract when its author explicitly marks it. Not every prose paragraph is a contract. The marker form is open (see Open Questions) but at minimum it must:

- Be visible in the rendered markdown without breaking reading flow.
- Carry the stable ID.
- Survive doc reorganization without breaking claims.
- Be greppable from the tool.

The likely shape is an HTML comment near the section header (`<!-- coverage: architecture.action_service_as_mutation_gate -->`) or a one-line frontmatter-style pin. The decision is deferred to the spec.

### 3. Every Test Claims at Least One Contract

Lua harness scripts gain a required `covers` field in their frontmatter. The field lists one or more contract IDs.

```
-- @covers: architecture.action_service_as_mutation_gate
-- @covers: cmdk.requirements.exhaustive_command_registry
```

Rust integration tests claim contracts via a mechanism to be settled in the spec (attribute macro, sidecar manifest, or doc-comment marker). Whatever the mechanism, it must be uniform within Rust and resolvable to the same ID space.

A test without any `covers` claim is rejected by the loader. A claim referencing an unknown ID is rejected by the loader. Both are loud failures, not warnings.

### 4. Missing Coverage is a Build Break

A registered contract with zero claims fails the build. This is the load-bearing requirement. Without it, the registry is documentation, not enforcement.

In the v1 tool-only stage, the build break is mediated by `brokkr coverage` (or whatever subcommand owns this) returning a non-zero exit code on uncovered contracts, and `brokkr check` invoking it. In the end-state, the check is promoted to compile time.

### 5. Architectural Enforcement is the End-State

The end-state of the coverage system is that the Rust compiler enforces the relationship between contracts and tests. The architecture doc states the guiding principle the system follows:

> Correctness should not depend on every developer remembering a multi-step protocol. A single entry point, a type that enforces the invariant, or a compiler error when the protocol is violated - these are how contracts become real.

The end-state shape:

- `ContractId` is a Rust enum or const table. Adding a contract is a code change the compiler tracks.
- A build script discovers Lua harness scripts, parses their frontmatter, and emits a generated `const LUA_TESTS: &[LuaTest]` table. Each entry carries `covers: &[ContractId]`.
- Compile-time assertions verify every `ContractId` variant appears in at least one `LuaTest::covers` and every claim resolves to a valid variant.
- Doc sections marked as contracts must have matching variants; a doc-lint step asserts the two sides agree.

What the architecture cannot enforce: that the Lua body actually exercises the claimed behavior. The compiler can enforce registration, claim, and coverage of the catalog. The author still has to write a real test. This is the same ceiling `CommandId` accepts today: exhaustive match arms in `update()` do not force each arm to do the right thing, but exhaustive registration plus exhaustive handling is most of the value.

### 6. Staged Delivery: Tool First, Codegen Later

The v1 implementation is a tool: a `brokkr` subcommand that reads doc anchors, parses Lua frontmatter, parses Rust test claims, and reports missing, unknown, and uncovered contracts. It fails the build on violations. It has the same data shape and same IDs as the end-state.

The v2 implementation promotes verification into the build system. The Rust enum, the build-script discovery, the compile-time asserts. Same catalog, same claims, stronger enforcement.

The staging is deliberate. The tool stage is the bridge that lets us populate the catalog and backfill claims on the existing test corpus before paying the build-time cost. Once the catalog is stable, codegen is mechanical.

### 7. Rust Integration Tests Are First-Class

Several architectural contracts are enforced by Rust integration tests, not Lua scripts. The crate-boundary lockdown tests in `crates/service-state/tests/` are the canonical example. These tests must claim contracts on the same footing as Lua scripts.

A separate class of contract is enforced by the compiler directly: typed IDs (`FolderId`, `TagId`), the `#[must_use]` discipline on `GenerationCounter::next()`, the `ProviderOps` trait. These contracts still appear in the registry. Their `Enforced-by:` field names the compile mechanism. They do not require a test claim. They appear in reports so the registry is honest about what is and is not test-covered.

### 8. No Grace Mode

There is no "warn only" period. When the system lands, every existing test is backfilled with a `covers` claim, every architectural boundary section is registered, every numbered Core Requirement in subsystem problem-statements is registered, and uncovered contracts that exist on day one are either covered immediately, retired, or marked as known gaps with a tracking issue.

This is the most expensive part of the rollout. It is also the only way the system means anything. A grace mode means tests written during the grace window have no claims, which means the catalog is not exhaustive, which defeats the entire principle.

The backfill is staged by doc area so it does not block on a single giant commit. Each area is brought to exhaustive coverage before the loader is hardened for that area's tests.

## Non-Goals

- Replacing test assertions with metadata. The test still has to prove the behavior. Frontmatter only records the claim.
- Building a parallel TOML or YAML requirements catalog. The docs are the registry.
- Measuring Rust line coverage or UI pixel coverage.
- Solving CI scheduling, flake quarantine, or expensive-test gating. Coverage is orthogonal.
- A coverage axis matrix in v1. Axes are deferred. The doc location is the only axis the v1 tool surfaces.
- Forcing every test into a one-test-per-contract shape. Many tests claim many contracts. Many contracts have many tests. The relationship is many-to-many.

## Decisions

1. **Exhaustive catalog.** Every behavioral contract is registered. The missing-coverage report is the derived diff between registered contracts and claimed contracts.

2. **Doc-anchored IDs.** Contract IDs are stable slugs rooted at doc sections. There is no parallel registry file. The docs are the catalog.

3. **No grace mode.** Backfill on rollout. After rollout, malformed tests are rejected by the loader and uncovered contracts fail the build.

4. **Tool-first, codegen-later.** v1 is a `brokkr` subcommand that performs the checks externally. v2 promotes verification to compile time via codegen and build-time assertions. Same data, same IDs.

5. **Axes deferred.** v1 has no coverage matrix. Doc location is the v1 axis. Cross-cutting matrix views are revisited when the catalog has earned its keep.

6. **Many-to-many.** A test may claim multiple contracts. A contract may have multiple claimants. The system does not enforce a particular cardinality.

7. **Rust tests are first-class claimants.** The claim mechanism in Rust is to be settled in the spec, but Rust integration tests sit on the same footing as Lua harness scripts.

## Open Questions

1. **What marks a doc section as a contract?** Options include an HTML comment near the section header (`<!-- coverage: id -->`), a one-line pin in a sidecar file, or a slug convention on the section header itself. Decision affects how the tool finds anchors and how stable IDs survive doc edits.

2. **Where does the `ContractId` enum live?** Candidate: a new `coverage` crate in the workspace whose build script discovers harness scripts and generates the enum. Affects how Rust integration tests reference the IDs.

3. **How do Rust integration tests claim contracts?** Attribute macro on test functions, sidecar manifest per crate, or a structured doc comment. Lua frontmatter is settled. The Rust analogue is not.

4. **How are compile-enforced contracts represented?** A contract enforced by typed IDs or `#[must_use]` has no test claim, only a description of the compile mechanism. The schema needs a way to distinguish "no claim because untested" from "no claim because the compiler proves it."

5. **What is the codegen trigger?** A `build.rs` in the coverage crate that re-emits when docs or harness scripts change, or a `brokkr` step that writes a checked-in file. Affects iteration speed and CI determinism.

6. **How are retired contracts handled?** A `retired` marker keeps the ID reserved so old test runs and external references do not break. Equivalent to keeping a `CommandId` variant for persistence compat after the feature is gone.

7. **How does the rollout sequence the backfill?** Per doc area, per crate, per harness directory. The smallest area large enough to validate the model is probably the command palette subsystem, which has clearly enumerated Core Requirements and a contained test surface.

## Success Criteria

The coverage system is working when:

- Every architectural boundary section in `docs/architecture.md` has a stable ContractId.
- Every numbered Core Requirement in subsystem problem-statements has a stable ContractId.
- Every Lua harness script claims at least one ContractId.
- Every Rust integration test that backs a contract claims at least one ContractId.
- A developer can run one command and get a precise list of registered contracts with no claims.
- A test added without a `covers` claim fails to load.
- A claim referencing an unknown ID fails to load.
- A contract added to the registry without at least one claim fails the build.

The end-state is reached when the Rust compiler enforces the four conditions above directly, with no separate tool invocation. The tool stage is the bridge that gets us there.
