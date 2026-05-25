# Contributing To OpenThymos

OpenThymos is an execution substrate for governed machine cognition. Changes
are accepted when they preserve the runtime contract: cognition proposes,
policy constrains, tools execute through typed boundaries, and the ledger
records the authoritative history.

## Contribution Principles

Correctness in the compiler, policy, writ, and ledger path is mandatory. A
feature that bypasses this path is a protocol regression, even if it improves
short-term ergonomics.

Contributions should prefer:

- small protocol changes with explicit invariants
- deterministic tests for compiler and replay behavior
- typed tool contracts over informal command strings
- append-only audit records over mutable status fields
- provider adapters that preserve runtime semantics
- documentation that distinguishes implemented behavior from future work

Contributions should avoid:

- hidden execution paths
- prompt-only safety claims
- unledgered approvals
- implicit tool authority
- provider-specific runtime behavior
- changes that make replay depend on fresh model output

## Local Verification

Run the project checks before opening a pull request:

```bash
npm run doctor
npm run verify
cd thymos
cargo test --workspace
```

For runtime changes, add focused Rust tests in the crate that owns the
behavior. Ledger, compiler, policy, and writ changes should include negative
tests for rejected or invalid states.

## Pull Request Requirements

A pull request that changes runtime semantics must state:

- which invariant is introduced, removed, or modified
- which ledger entries are emitted or consumed
- whether replay behavior changes
- whether capability writ validation changes
- whether provider behavior becomes visible to runtime semantics
- which tests prove the new behavior

Use [RFC_TEMPLATE.md](RFC_TEMPLATE.md) for changes that affect protocol shape,
ledger format, writ structure, provider contracts, replay semantics, or policy
evaluation.

## Code Boundaries

| Area | Rule |
| --- | --- |
| `thymos-core` | Protocol types must remain serializable, content-addressable, and version-aware. |
| `thymos-compiler` | Compilation must reject invalid authority before tool visibility. |
| `thymos-policy` | Policies should be pure functions over intent, writ, and world view. |
| `thymos-ledger` | Entries are append-only and parent-chained. Updating prior entries is invalid. |
| `thymos-runtime` | Tool execution must occur only after a staged proposal. |
| `thymos-cognition` | Providers produce intents. They do not execute tools or persist state. |
| `thymos-tools` | Tools must declare schemas, costs, preconditions, postconditions, and structured deltas. |

## Documentation Standard

Documentation should read like systems documentation. Use precise terms:
runtime, substrate, execution system, policy engine, capability writ, ledger,
projection, replay, provider adapter, commit.

Avoid generic product language. Describe OpenThymos as runtime infrastructure,
not as a model wrapper or chat surface. When a guarantee is aspirational rather
than implemented, label it as roadmap material.

## Governance

Project decisions follow [GOVERNANCE.md](GOVERNANCE.md). Protocol changes are
made through RFCs. Maintainers may request an RFC for any change that alters
runtime semantics or long-term compatibility.

## License

By contributing, you agree that your contribution is licensed under the
Apache-2.0 license.
