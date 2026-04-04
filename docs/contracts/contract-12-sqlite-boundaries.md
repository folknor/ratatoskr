# Contract #12: SQLite Boundaries — Problem Statement

## Problem

`rusqlite` currently leaks across more of the codebase than the architecture wants to allow. If `app` is presentation-only, `rtsk` is the business-logic facade, and storage crates own persistence details, then low-level SQLite coupling should be concentrated near those storage boundaries rather than spread across feature and domain code. Broad `rusqlite` exposure makes it too easy for crates to bypass intended abstractions and encode persistence behavior directly where higher-level logic should live.

The problem is not simply dependency hygiene for its own sake. Every additional crate that depends directly on `rusqlite` weakens the architectural boundary between storage mechanics and domain behavior. It becomes harder to see which crate owns query shape, transaction scope, row mapping, migration assumptions, and database invariants. That in turn makes refactors riskier and encourages feature work to grow around existing SQL call sites instead of around stable contracts.

This contract needs to define which crates are allowed to depend on `rusqlite` directly, which crates must instead depend on higher-level storage APIs, and what migration path will push existing direct usage downward. Until those boundaries are explicit, SQLite will remain a cross-cutting implementation detail instead of an owned subsystem.
