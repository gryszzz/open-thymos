# Interfaces

Thymos has multiple operator surfaces, but they are all clients of the same backend runtime.
They share the same run state, capability registry, approvals, and ledger
projection.

## Web console

Best for onboarding, execution logs, world replay, and operator visibility.

## CLI

Best for terminal-first users who want to launch, follow, inspect, diff, resume, and cancel runs.

## VS Code sidebar

Best for editor-native monitoring and approval review.

## Shell / system terminal

Best for users who want a persistent terminal workflow attached to Thymos.

## Shared across all of them

- run id
- execution phase
- registered capabilities
- operator state
- counters
- live execution log
- final answer
