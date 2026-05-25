---
layout: default
title: Policy Intercept Demo
eyebrow: Demo
subtitle: A dangerous action is suspended, approved, resumed, committed, and replayed.
permalink: /demos/policy-intercept-approval/
---

# Policy Intercept Demo

This showcase demonstrates the execution shape OpenThymos is built for:

1. cognition proposes a dangerous action
2. the policy engine intercepts the proposal
3. a human approval is required
4. execution resumes after approval
5. the entire run remains replayable
6. ledger folding reproduces state exactly

The demo is intentionally serious. The point is not that the model is
obedient. The point is that the model is not the authority.

## Scenario

An operator asks OpenThymos to rotate a production deploy key after a suspected
credential leak. The agent attempts to perform an irreversible external action:
revoking the active key and publishing the replacement.

The writ permits repository inspection, key generation, and local verification.
It does not allow irreversible external publication without maintainer
approval.

## Architecture Walkthrough

```text
Intent:
  target = "secrets_publish"
  args   = { "environment": "prod", "irreversible": true }

Compiler:
  validates writ signature
  checks tool scope
  checks budget
  validates args
  evaluates policy

Policy:
  decision = RequireApproval
  channel  = "maintainer/security"
  reason   = "irreversible external credential rotation"

Ledger:
  appends PendingApproval

Operator:
  approves proposal id prop_c4b1

Runtime:
  resumes proposal
  executes tool
  records observation
  commits structured delta

Replay:
  verifies hash chain
  folds final state
  reproduces the same world hash
```

## CLI Session

```bash
$ thymos run "rotate the production deploy key after verifying the repo state" \
    --provider anthropic \
    --writ writ_security_2026_05 \
    --follow
```

```text
[system] run accepted
         run_id=run_847 trajectory=traj_b4e2f9187a6c0e11

[intent] step=1 tool=repo_map
         rationale="identify deployment and secret-management surfaces"

[proposal] staged proposal=prop_5d1c tool=repo_map
           policy=writ.authority,tenant.isolation decision=permit

[execution] repo_map observed delta_ops=1 latency=42ms

[result] commit seq=1 commit=commit_10a99f

[intent] step=2 tool=secrets_generate
         rationale="generate replacement deploy key locally"

[proposal] staged proposal=prop_8a02 tool=secrets_generate
           policy=writ.authority,tenant.isolation decision=permit

[execution] secrets_generate observed delta_ops=2 latency=88ms

[result] commit seq=2 commit=commit_d91f22

[intent] step=3 tool=secrets_publish
         rationale="publish replacement key and revoke old key"

[proposal] suspended proposal=prop_c4b1 tool=secrets_publish
           channel=maintainer/security
           reason="irreversible external credential rotation"

[status] waiting_approval
         proposal=prop_c4b1 channel=maintainer/security
```

The runtime has not published the secret. It has recorded the proposed action
and stopped at the authority boundary.

## Approval

```bash
$ thymos runs show run_847
```

```text
pending approvals for run_847

proposal: prop_c4b1
tool:     secrets_publish
channel:  maintainer/security
reason:   irreversible external credential rotation
writ:     writ_security_2026_05
effects:  external=true irreversible=true
status:   suspended
```

```bash
$ thymos approve run_847 maintainer/security
```

```text
[approval] granted proposal=prop_c4b1 channel=maintainer/security
[execution] secrets_publish observed delta_ops=2 latency=341ms
[result] commit seq=5 commit=commit_f2e4b7
[status] resumed
```

## Runtime Event Stream

```json
[
  {
    "phase": "intent",
    "event": "intent_declared",
    "step": 3,
    "tool": "secrets_publish",
    "intent_id": "intent_3f8c"
  },
  {
    "phase": "proposal",
    "event": "proposal_suspended",
    "proposal_id": "prop_c4b1",
    "policy": {
      "rules_evaluated": [
        "writ.authority",
        "tenant.isolation",
        "irreversible.approval"
      ],
      "decision": {
        "kind": "require_approval",
        "channel": "maintainer/security",
        "reason": "irreversible external credential rotation"
      }
    }
  },
  {
    "phase": "proposal",
    "event": "approval_resolved",
    "proposal_id": "prop_c4b1",
    "approved": true
  },
  {
    "phase": "execution",
    "event": "execution_observed",
    "tool": "secrets_publish",
    "latency_ms": 341,
    "delta_ops": 2
  },
  {
    "phase": "result",
    "event": "commit_recorded",
    "commit_id": "commit_f2e4b7",
    "seq": 5
  }
]
```

## Screenshot Concepts

The operator console should show three panels:

| Panel | Contents |
| --- | --- |
| Proposal | tool, args digest, writ id, effect class, policy trace |
| Authority | scope match, budget, tenant, time window, effect ceiling |
| Replay | ledger sequence, world hash before approval, world hash after commit |

The approval control should operate on `proposal_id`, not on free text.

## Replay Demonstration

```bash
$ thymos replay run_847 --verify --fold-world --policy-trace
```

```text
[integrity] hash_chain=ok parent_chain=ok sequence=ok

[policy] seq=3 proposal=prop_c4b1
         decision=require_approval
         channel=maintainer/security
         reason="irreversible external credential rotation"

[approval] proposal=prop_c4b1 approved=true

[fold] seq=1 commit=commit_10a99f tool=repo_map          world=0e2b347d
[fold] seq=2 commit=commit_d91f22 tool=secrets_generate world=745b9e10
[fold] seq=5 commit=commit_f2e4b7 tool=secrets_publish  world=a9014cc2

[report] final_world_hash=a9014cc2e1d44ef8d0927a15a17fb810
[result] replay verified
```

The replay report proves that the dangerous action did not occur until after
the recorded approval and that the final state is reproduced from ledgered
facts.
