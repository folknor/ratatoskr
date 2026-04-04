# Contract #11: Calendar Architecture — Problem Statement

## Problem

Calendar behavior has been improved incrementally, but the feature still lacks a clearly enforced architectural contract. Event detail, event editing, deletion, seeded data, and UI surfaces now work well enough to exercise the feature, but the system still feels like a set of working slices rather than a coherent calendar architecture. That increases the risk that future fixes will be local, correct-looking patches that further entangle state, UI behavior, and data flow.

The core issue is not just missing features or papercuts. It is that calendar currently sits in an in-between state: substantial enough to need real architectural boundaries, but not yet constrained enough that those boundaries are obvious in the code. Surface state, event lifecycle, account/calendar ownership, editing flows, and persistence responsibilities need a clearer contract so new calendar work cannot silently reintroduce ad hoc coupling between view state, editing state, and storage behavior.

This contract needs to define what the calendar feature's stable boundaries are: what state belongs to UI presentation, what belongs to feature/domain logic, what transitions are first-class, and how event identity and account ownership flow through the system. Until that is explicit, calendar work will continue to succeed tactically while remaining fragile strategically.
