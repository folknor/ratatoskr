# Coverage Marker Spec

Coverage contracts are registered in the docs, next to the section that defines
the contract. The docs are the catalog; there is no sidecar registry.

The marker is an HTML comment immediately after the markdown heading. Blank
lines between the heading and marker are allowed. Prose, lists, code blocks, or
other content between the heading and marker are rejected.

```markdown
### Action service as mutation gate
<!-- coverage: architecture.action_service_as_mutation_gate enforcement=rust-test -->
```

If a section needs more than one contract ID, place the marker lines together
directly below the heading.

The first token after `coverage:` is the stable contract ID. IDs are lowercase
dotted slugs. Each segment starts with a letter and may contain lowercase
letters, digits, or underscores.

Optional metadata uses `key=value` tokens:

- `status=required` is the default.
- `status=gap` registers a known missing test claim.
- `status=retired` reserves an old ID without requiring active coverage.
- `status=compile-enforced` marks a contract whose enforcement is represented
  by compiler behavior rather than a runtime test.
- `enforcement=compiler`, `rust-test`, `lua-harness`, `convention`, or `mixed`
  records the current enforcement shape.

`status=gap`, `status=retired`, `status=compile-enforced`, and
`enforcement=compiler` are visible in reports but do not require a Lua claim in
the read-only report. They still remain registered contracts.

The slice 1 parser lints:

- duplicate contract IDs
- malformed markers
- invalid ID grammar
- invalid metadata
- markers with no preceding markdown heading
- markers that are not immediately below their heading

Markers inside fenced code blocks are ignored so documentation can show examples
without registering duplicate contracts.

## Lua Claims

Lua harness scripts claim contracts in the frontmatter comment block with one
`@covers` line per contract:

```lua
-- description: JMAP initial sync imports the small fixture
-- @covers: architecture.folder_vs_label_semantics_are_explicit
-- @covers: sync.jmap_initial_import
-- fixture: jmap-small.toml
```

Each line names exactly one contract ID. Comma-separated lists and multiple IDs
on one line are rejected. Claims are read only from the initial comment
frontmatter, before the Lua body starts.

The current parser validates Lua claim syntax and the report surfaces missing
claims and unknown IDs. It does not make missing claims a hard loader error yet.

## Commands

The workspace crate exposes a small process entrypoint for the tool-first stage:

- `ratatoskr-coverage lint-docs [DOCS_ROOT]` lints doc markers only.
- `ratatoskr-coverage report [WORKSPACE_ROOT] [--area ID_PREFIX] [--strict]`
  prints registered contracts, doc diagnostics, Lua claim diagnostics,
  registered contracts with no Lua claim, Lua tests with no claim, and Lua
  claims that reference unknown IDs. `--area` filters contract and claim lists
  by contract ID prefix and skips the global no-claim test list. `--strict`
  exits non-zero for doc diagnostics, Lua claim syntax diagnostics, unknown Lua
  claims, uncovered contracts, and, when no area is selected, Lua tests without
  any claim.

The folders-labels pilot is additionally enforced by the coverage crate's
repository smoke tests. `brokkr check -p coverage` fails if
`glossary.folders_labels` gains an uncovered contract or an unknown Lua claim.

The eventual `brokkr coverage` command can shell out to this entrypoint or
reimplement the same parser on the brokkr side. That integration point remains
an explicit cross-repository decision because brokkr is not a Ratatoskr crate.
