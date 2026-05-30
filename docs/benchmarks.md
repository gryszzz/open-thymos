---
layout: default
title: Benchmarks
eyebrow: Measurement protocol
subtitle: Benchmark the execution substrate, not provider cleverness.
permalink: /benchmarks/
---

# Benchmarks

OpenThymos benchmarks should measure runtime properties that affect governed
execution. Provider model quality is outside this benchmark suite.

## Reporting Requirements

Every benchmark report must include:

- OpenThymos git commit
- compiler version
- storage backend
- operating system
- CPU model
- memory size
- disk type
- provider mode
- tool registry size
- ledger entry count
- commit count
- warm or cold run
- sample size
- percentile summary

## Benchmark Matrix

| Benchmark | Unit | Primary Question |
| --- | --- | --- |
| Replay speed | entries/sec | How quickly can a ledger be verified and folded? |
| Execution overhead | ms/proposal | How much time does runtime governance add before tool execution? |
| Provider swap latency | ms/init | How quickly can a run select and initialize a provider adapter? |
| Ledger folding performance | commits/sec | How quickly can committed deltas rebuild world state? |
| Tool execution latency | ms/tool | What latency is spent inside tool contracts? |
| State projection speed | resources/sec | How quickly can the current world be projected? |
| Execution DAG traversal | nodes/sec | How quickly can delegated trajectories be traversed? |
| Memory usage | MiB | How much memory is consumed by replay and projection? |

## Standard Workloads

### `ledger-small`

- 100 entries
- 60 commits
- 5 pending approvals
- 5 rejections
- 1 branch
- 1 delegation edge

### `ledger-medium`

- 10,000 entries
- 7,000 commits
- 500 pending approvals
- 500 rejections
- 100 branches
- 200 delegation edges

### `ledger-large`

- 1,000,000 entries
- 700,000 commits
- 25,000 pending approvals
- 25,000 rejections
- 5,000 branches
- 10,000 delegation edges

## Phase I Baseline Report

Measured on the first complete Phase I implementation commit. These are the
numbers to beat. Run the ignored benchmark tests with:

```bash
cargo test -p thymos-ledger --features sqlite bench -- --include-ignored --nocapture
cargo test -p thymos-runtime bench_execution_overhead -- --include-ignored --nocapture
```

```text
OpenThymos benchmark report
git:              6cf13d4  (Phase I baseline)
compiler:         thymos-compiler/0.0.1
backend:          sqlite (in-memory)
host:             macOS arm64
provider:         mock (KvSetTool, no network)
workload:         ledger-small (1 root + 1000 commits)
samples:          5 (warm runs; each run full replay)

replay_speed (hash verify + parent chain + world fold):
  avg: ~12,400 entries/sec

ledger_folding (delta application only, no hash verify):
  avg: ~656,000 commits/sec

execution_overhead (compile + policy + tool execute + ledger append):
  avg: ~1.35 ms/proposal  (~740 proposals/sec)
```

### Interpretation

The ~53× gap between folding speed and replay speed reflects hash verification
cost: replay recomputes `blake3(canonical_json(payload))` for every entry
while folding only applies `DeltaOp` variants to `World`. The hash verification
cost scales with the number of entries, not commits alone, so ledgers with many
non-commit entries (rejections, approvals) will show a larger gap.

Execution overhead of ~1.35 ms/proposal includes: world projection from ledger,
compile (writ check + policy eval + type check), KvSet tool execution (in-memory
BTreeMap write), delta trial-apply, and SQLite append. Network-backed tools will
add provider latency on top.

## Example Report Format

```text
OpenThymos benchmark report
git:              <commit>
compiler:         thymos-compiler/0.0.1
backend:          sqlite | postgres
host:             macOS arm64 | linux x86_64
cpu:              Apple M4 Max | Intel Xeon Gold 6154
memory:           64 GiB
provider:         mock | anthropic | openai
workload:         ledger-small | ledger-medium | ledger-large
samples:          30
warmup:           5

replay_speed:
  p50: <n> entries/sec
  p95: <n> entries/sec
  p99: <n> entries/sec

execution_overhead:
  p50: <n> ms/proposal
  p95: <n> ms/proposal
  p99: <n> ms/proposal

ledger_folding:
  p50: <n> commits/sec
  p95: <n> commits/sec
  p99: <n> commits/sec

state_projection:
  p50: <n> resources/sec
  p95: <n> resources/sec
  p99: <n> commits/sec

memory_usage:
  peak_rss: <n> MiB
```

## Benchmark Commands

Planned benchmark command shape:

```bash
thymos bench replay --workload ledger-medium --backend sqlite --samples 30
thymos bench fold --workload ledger-large --backend postgres --samples 10
thymos bench provider-swap --providers mock,openai,anthropic --samples 50
thymos bench dag --workload delegation-medium --samples 30
```

Until a dedicated benchmark harness lands, contributors should use Criterion
for Rust crate-level measurements and include raw command output in pull
requests that claim performance changes.

## Methodology

Benchmark runtime paths independently:

- isolate provider initialization from provider generation
- isolate compiler overhead from tool execution
- isolate ledger read speed from delta application
- isolate DAG traversal from child trajectory replay
- report cold cache and warm cache separately

Do not benchmark a full hosted model loop and present it as runtime overhead.
That measurement mostly reflects provider latency.

## Regression Policy

A regression in ledger, replay, compiler, or projection performance should be
treated as a runtime concern when it exceeds:

- 10 percent for p50 latency
- 20 percent for p95 latency
- 10 percent for peak memory
- any change that makes replay non-linear without documented cause

Performance changes that alter semantics require an RFC.
