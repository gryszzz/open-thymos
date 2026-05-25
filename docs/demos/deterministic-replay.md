---
layout: default
title: Deterministic Replay Demo
eyebrow: Demo
subtitle: A realistic replay of run_847 through the execution ledger.
permalink: /demos/deterministic-replay/
---

# Deterministic Replay Demo

This demo shows the operator experience for:

```bash
thymos replay run_847
```

The run has already completed. Replay does not call the provider and does not
execute tools. It verifies ledger structure, replays policy-visible decisions,
folds committed deltas, and reproduces the final world projection.

## Scenario

Run `run_847` inspected a repository, patched one file, ran tests, hit a policy
approval boundary, resumed after approval, and committed the final state.

The ledger contains:

- one root entry
- four committed tool observations
- one rejected proposal
- one pending approval
- one approval resolution event represented through resumed commit path
- one final commit

## Command

```bash
$ thymos replay run_847 --verify --fold-world --policy-trace
```

## Terminal Output

```text
OpenThymos replay
run:              run_847
trajectory:       traj_b4e2f9187a6c0e11
ledger:           /var/lib/thymos/thymos-ledger.db
mode:             verify + fold-world + policy-trace

[00] load
     entries: 8
     first:   seq=0 kind=root id=8f42b0a1
     head:    seq=7 kind=commit id=6ad19c3e

[01] integrity
     hash chain:        ok
     parent chain:      ok
     sequence:          ok
     canonical payload: ok

[02] policy-visible entries
     seq=1 proposal=prop_5d1c tool=repo_map
       decision: committed proposal id recorded
     seq=2 proposal=prop_91aa tool=fs_read
       decision: committed proposal id recorded
     seq=3 proposal=prop_7b30 tool=fs_patch
       rules: writ.authority, tenant.isolation, threshold.approval
       decision: require_approval(channel=maintainer, reason="write_set = 14 exceeds threshold 8")
     seq=4 rejection intent=intent_c012
       reason: policy_denied("attempted external publish without writ scope")
     seq=6 proposal=prop_7b30 tool=fs_patch
       approval: granted(channel=maintainer)
       decision: permit_after_approval
     seq=7 proposal=prop_00f4 tool=test_run
       decision: committed proposal id recorded

[03] fold
     commit seq=1 tool=repo_map  delta_ops=1  world_hash=0e2b347d
     commit seq=2 tool=fs_read   delta_ops=1  world_hash=29ac7712
     commit seq=5 tool=fs_patch  delta_ops=3  world_hash=c99407aa
     commit seq=7 tool=test_run  delta_ops=1  world_hash=91fd22cb

[04] report
     entries seen:        8
     commits replayed:   4
     policy outcomes:      5
     approvals replayed:  1
     rejected proposals:  1
     head sequence:       7
     head commit:         commit_6ad19c3e
     compiler versions:   thymos-compiler/0.1.0
     final world hash:    91fd22cb4fd8332ef728c912c50e1f8e

result: replay verified
```

## Replay Log

```json
{
  "run_id": "run_847",
  "trajectory_id": "traj_b4e2f9187a6c0e11",
  "verification": {
    "hash_chain": "ok",
    "parent_chain": "ok",
    "sequence": "ok",
    "compiler_version": "ok"
  },
  "fold": [
    {
      "seq": 1,
      "tool": "repo_map",
      "commit": "commit_10a99f",
      "delta_ops": 1,
      "world_hash": "0e2b347d"
    },
    {
      "seq": 2,
      "tool": "fs_read",
      "commit": "commit_30df91",
      "delta_ops": 1,
      "world_hash": "29ac7712"
    },
    {
      "seq": 5,
      "tool": "fs_patch",
      "commit": "commit_893e0b",
      "delta_ops": 3,
      "world_hash": "c99407aa"
    },
    {
      "seq": 7,
      "tool": "test_run",
      "commit": "commit_6ad19c3e",
      "delta_ops": 1,
      "world_hash": "91fd22cb"
    }
  ],
  "report": {
    "entries_seen": 8,
    "commits_replayed": 4,
    "head_seq": 7,
    "head_commit": "commit_6ad19c3e",
    "final_world_hash": "91fd22cb4fd8332ef728c912c50e1f8e"
  }
}
```

## Architecture Notes

Replay uses the same ledger semantics as runtime projection:

```text
ledger entries -> integrity verifier -> commit filter -> delta fold -> world
```

The replay engine proves:

- the same tool calls are represented by the same committed observations
- the same runtime state is reconstructed by the same deltas
- the same ledger fold produces the same world hash
- pending approvals retain their recorded proposal policy traces
- approval suspension is visible as ledger state
- provider output is not needed to reproduce historical state

## Failure Example

Compiler pinning can detect version drift:

```bash
$ thymos replay run_847 --verify --require-compiler thymos-compiler/9.9.9
```

```text
[03] fold
     error: compiler version drift at commit commit_10a99f
     pinned: thymos-compiler/9.9.9
     found:  thymos-compiler/0.1.0

result: replay rejected
```

This is the expected behavior. A replay system should fail loudly when a
historical trajectory cannot be verified under the requested runtime rules.
