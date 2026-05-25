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

## Example Report

```text
OpenThymos benchmark report
git:              4f91a2c
compiler:         thymos-compiler/0.1.0
backend:          sqlite
host:             macOS arm64
cpu:              Apple M3 Max
memory:           64 GiB
provider:         mock
workload:         ledger-medium
samples:          30
warmup:           5

replay_speed:
  p50: 188,400 entries/sec
  p95: 171,900 entries/sec
  p99: 165,200 entries/sec

execution_overhead:
  p50: 1.8 ms/proposal
  p95: 3.9 ms/proposal
  p99: 6.2 ms/proposal

ledger_folding:
  p50: 94,000 commits/sec
  p95: 88,700 commits/sec
  p99: 81,300 commits/sec

state_projection:
  p50: 212,000 resources/sec
  p95: 198,000 resources/sec
  p99: 190,500 resources/sec

memory_usage:
  peak_rss: 287 MiB
```

Numbers above are an example format, not a release claim.

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
