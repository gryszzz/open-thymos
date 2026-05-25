# OpenThymos RFC

## Title

Short noun phrase naming the protocol change.

## Status

Draft | Accepted | Rejected | Superseded

## Summary

Describe the change in one or two paragraphs. State whether this affects
runtime semantics, ledger compatibility, replay, writs, policy, providers,
tool contracts, or operator surfaces.

## Motivation

What failure mode or missing capability does this address? Explain the system
pressure without relying on product language.

## Current Semantics

Describe the existing runtime behavior. Include relevant type names, ledger
entry kinds, compiler stages, policy decisions, or writ fields.

## Proposed Semantics

Define the new behavior precisely. If order matters, specify order. If identity
or hashing changes, specify the canonical input.

## Invariants

List the invariants that must hold after this change. Include negative
statements where useful, such as "a provider MUST NOT execute tools".

## Ledger Impact

State whether the change introduces, removes, or modifies ledger entries.
Include replay implications and compatibility notes for existing ledgers.

## Replay Impact

Explain how replay verifies the new behavior. If replay cannot verify part of
the behavior, state why and identify the trust boundary.

## Writ And Policy Impact

Describe changes to capability writ validation, policy evaluation, approval
flows, budgets, time windows, tenant boundaries, or effect ceilings.

## Provider Impact

Describe whether cognition providers need new fields, adapters, serialization
rules, or compatibility behavior. Providers must not gain execution authority.

## Tool Contract Impact

Describe changes to tool schemas, preconditions, postconditions, estimated
costs, structured deltas, or observed output.

## Compatibility

Specify:

- compatible versions
- incompatible versions
- migration procedure
- data that cannot be migrated
- how old ledgers are replayed after the change

## Security Considerations

Identify authority bypass risks, policy bypass risks, replay divergence risks,
ledger forgery risks, and provider inconsistency risks.

## Alternatives

List meaningful alternatives and why they were rejected.

## Test Plan

List unit, integration, replay, and negative tests required before
implementation can be considered complete.

## Unresolved Questions

List remaining design questions. Do not merge an RFC with unresolved questions
that affect compatibility or replay correctness.
