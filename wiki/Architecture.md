# Architecture

Thymos separates proposing work from executing work, then exposes the same
execution truth across CLI, VS Code, terminal, and web surfaces.

## Flow

`Cognition -> Intent -> Proposal -> Commit`

## What the runtime owns

- authority
- policy checks
- capability execution
- sandbox boundaries
- failure handling
- logging
- completion state

## Why that matters

Because the runtime owns execution truth, multiple interfaces can attach to the same run without diverging.

## Main pieces

- `thymos-cognition`
- `thymos-runtime`
- `thymos-tools`
- `thymos-worker`
- `thymos-policy`
- `thymos-ledger`
- `thymos-server`
- `thymos-cli`
