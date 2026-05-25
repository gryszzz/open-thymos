# OpenThymos Governance

OpenThymos is governed as infrastructure. The project favors explicit
protocols, written invariants, and durable review history over informal
roadmap pressure.

## Governing Object

The governed object is the OpenThymos runtime protocol:

- Intent, Proposal, Commit, Writ, World, and Ledger semantics
- deterministic replay behavior
- provider abstraction boundaries
- policy evaluation and approval semantics
- execution trace and audit records
- compatibility rules for persisted ledger data

Implementation details may change freely when they preserve these semantics.
Protocol changes require review under the process below.

## Maintainer Responsibilities

Maintainers are responsible for:

- preserving runtime invariants
- rejecting hidden execution paths
- requiring tests for ledger, compiler, policy, replay, and writ changes
- keeping documentation aligned with implemented behavior
- reviewing compatibility effects before accepting protocol changes
- maintaining the RFC record

Maintainer authority is custodial. It exists to protect the runtime contract,
not to accelerate feature volume.

## Decision Classes

### Class 0: Editorial

Documentation, comments, examples, naming cleanup, and non-semantic
reorganization. These changes require normal pull request review.

### Class 1: Implementation

Internal code changes that preserve runtime semantics. Examples include
performance work, refactors, additional tests, and isolated bug fixes. These
changes require tests when they touch execution paths.

### Class 2: Runtime Semantics

Changes to compiler ordering, writ validation, policy decisions, ledger entry
kinds, replay verification, provider contracts, tool contracts, or projection
rules. These changes require an RFC unless maintainers explicitly classify the
change as a bug fix.

### Class 3: Protocol Compatibility

Changes that affect persisted ledger interpretation, content hashes, canonical
serialization, writ format, commit format, or replay across versions. These
changes require an RFC, compatibility notes, migration guidance, and explicit
maintainer acceptance.

## RFC Process

1. Open an RFC using [RFC_TEMPLATE.md](RFC_TEMPLATE.md).
2. Define the invariant being introduced or changed.
3. Specify the impact on ledger format, replay, policy, writs, providers, and
   tools.
4. Document rejected alternatives.
5. Allow maintainer review and public comment.
6. Merge the RFC only after the compatibility story is clear.
7. Implement the RFC in one or more focused pull requests.

An RFC is a design record, not a marketing artifact. It should be precise
enough for a future maintainer to understand why the protocol took its shape.

## Compatibility Policy

OpenThymos treats persisted execution history as archival data. A release
should not invalidate prior ledgers without a documented migration path.

Compatibility-sensitive changes include:

- canonical JSON changes
- content hash input changes
- ledger entry kind changes
- commit body changes
- writ body changes
- replay verifier assumptions
- compiler version pinning behavior

When compatibility cannot be preserved, the project must document the break,
the affected versions, the replay consequences, and the migration mechanism.

## Security And Safety

The security boundary is the runtime path, not the model prompt. Reports that
show authority bypass, policy bypass, replay divergence, ledger corruption, or
untracked effects are treated as high priority.

Security fixes may bypass the public RFC waiting period, but the resulting
semantic change should still be documented after the fix lands.

## Release Discipline

Releases should include:

- runtime invariant changes
- ledger compatibility notes
- replay compatibility notes
- provider contract changes
- new or removed tool capabilities
- migration instructions when required

Version numbers must not imply protocol stability until the maintainers mark a
protocol surface as stable.
