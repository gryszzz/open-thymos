# Audit: is the agent really under control?

Date: 2026-06-06 · Scope: the cognition → runtime authority boundary. Verified
by reading the enforcement code paths and running the governance test suite +
`thymos audit` against a live runtime. This note captures the finding so it
isn't only in a chat log.

**Verdict:** Yes — control is enforced in code (a pure compiler + a single
execution chokepoint), not by prompt convention, and it is regression-tested
adversarially. The residual trust sits in **tool honesty, writ-key custody, and
your policy/approval configuration** — which is the correct place for it. See
[§ Honest trust boundaries](#honest-trust-boundaries).

---

## How the agent works

The loop ([`agent.rs`](../../thymos/crates/thymos-runtime/src/agent.rs)) is
deliberately minimal. Each step it:

1. projects the world from the ledger,
2. asks cognition for a batch of **intents** (`cognition.step(ctx)`),
3. routes each intent through `submit` → the compiler,
4. records the typed outcome and feeds it back next step.

**Cognition's authority is bounded by the type system, not by convention.** The
`Cognition` trait
([`cognition/lib.rs:99`](../../thymos/crates/thymos-cognition/src/lib.rs#L99))
exposes exactly one method, `step() -> CognitionStep`, and `CognitionStep`
carries only `{ intents, final_answer, usage }`. There is no method to execute a
tool, write the ledger, or mutate state — the model *cannot* do those things
because nothing hands it the capability. A model may *propose* anything;
proposing is harmless.

## Where control is enforced

Every intent passes through `compile_with_context`
([`compiler/lib.rs:115`](../../thymos/crates/thymos-compiler/src/lib.rs#L115)) —
a **pure function** that runs gates in a fixed order and returns either a
`Staged` proposal, a typed `Rejected`, or `Suspended` (needs approval):

| # | Gate | Reject reason |
|---|------|---------------|
| 1 | intent kind supported | `TypeMismatch` |
| 2 | writ signature valid | `AuthorityVoid` |
| 2b | writ (and parent) not revoked | `AuthorityVoid` |
| 3 | now ∈ writ time window | `AuthorityVoid` |
| 4 | tool ∈ writ scope | `AuthorityVoid` |
| 5 | tool exists | `UnknownTool` |
| 5b | **tool effect class ≤ writ effect ceiling** (Read ≤ Write ≤ External ≤ Irreversible) | `AuthorityVoid` |
| 6 | projected cost ≤ writ budget | `BudgetExhausted` |
| 7 | args typecheck | `TypeMismatch` |
| 8 | preconditions hold | `PreconditionFailed` |
| 9 | policy verdict | `PolicyDenied` / `RequireApproval` |
| 9b | irreversible & uncompensable ⇒ force human approval | suspends |

The single chokepoint that matters: in `submit`
([`runtime/lib.rs:600`](../../thymos/crates/thymos-runtime/src/lib.rs#L600)),
`tool.execute(&inv)` is reachable **only** inside the `Compiled::Staged` arm
([`runtime/lib.rs:645`](../../thymos/crates/thymos-runtime/src/lib.rs#L645)).
`Rejected` appends a rejection and returns; `Suspended` parks a pending approval
and returns. **There is no other path to execution.**

## Proof — gate tests (run 2026-06-06)

The control tests don't merely assert "an error came back." They register a
`PoisonTool` whose `execute()` increments a counter, then assert the counter
stayed **0** — i.e. a denied intent *provably never reached the tool*
([`compiler_rejection.rs`](../../thymos/crates/thymos-runtime/tests/compiler_rejection.rs),
`assert_never_executed`).

```
$ cargo test -p thymos-runtime --test compiler_rejection \
      --test cognition_budget --test revocation --test redaction \
      --test replay_safety --test quorum --test agent_loop

compiler_rejection ... 10 passed   (one per gate; all "rejects_before_execute_when_*")
  signature_invalid · time_window_expired · tool_outside_writ_scope · tool_unknown
  effect_exceeds_ceiling · budget_exhausted · args_fail_validation
  precondition_fails · policy_denies · poison_tool_unused_in_clean_run
cognition_budget   ...  2 passed   (model token/USD spend halts the run)
revocation         ...  3 passed   (revoked & parent-revoked writ rejected before execution)
redaction          ...  2 passed   (secrets scrubbed before hitting the ledger)
replay_safety      ...  4 passed   (replay never calls a provider or re-runs a tool)
quorum             ...  2 passed   (M-of-N approval; a single denial vetoes)
agent_loop         ...  6 passed   (end-to-end loop; rejections don't stop progress)
```

## Proof — `thymos audit` on a live runtime (run 2026-06-06)

A runtime was started on an isolated ledger (`THYMOS_LEDGER_PATH`), a run was
driven through the HTTP API, and audited:

```
$ thymos audit c4655c04-031e-41b5-8959-42e2dd361ea8

OpenThymos audit
run:              c4655c04-031e-41b5-8959-42e2dd361ea8
trajectory:       c085755abb491f3a388b385e0339779455e20c22847ec29ffd6400cf23db82e1

ledger (1 entries)
  #0   ROOT        trajectory bound  "Map the repo and summarize the runtime"

replay
  [integrity] verified
  commits replayed:   0
  rejected proposals: 0
  head sequence:      0
  final world hash:   f4cfe8821990755fea8de29b9ce6d107875717dea5f2886aa59f779d651544a8

result: 0 commits, 0 rejections — replay verified
```

This proves the audit + replay machinery works end to end on a real append-only
ledger: the trajectory is content-addressed, the chain integrity is verified,
and a world hash is reproduced by folding.

**Why the trail is short (honest caveat):** no provider key was set, so
`/health` reported `cognition_live: false` and the bundled cognition is the
deterministic mock. The server's mock is constructed with an **empty intent
script**
([`cognition/lib.rs:335`](../../thymos/crates/thymos-cognition/src/lib.rs#L335)),
so it proposes nothing and the run commits nothing — hence ROOT-only. A
commit-bearing, gate-firing trail (commits + a rejection + an irreversible
approval suspension) requires a real model: run the gated live path
(`ANTHROPIC_API_KEY=… cargo test -p thymos-runtime --test live_provider --
--ignored --nocapture`) or drive `thymos run` with a key set.

## Honest trust boundaries

Control means *no action without a signed, scoped, policy-checked grant* — proven
above. It does **not** mean the following are guaranteed; these are the residual
trust assumptions:

- **A tool self-declares its `effect_class`.** The ceiling check (gate 5b) trusts
  that an `Irreversible` tool isn't mislabeled `Read`. A malicious/buggy tool
  that lies about its effect class would slip under the ceiling. The root of
  trust for tool honesty is **signed manifests + the marketplace**, not the
  compiler.
- **Whoever holds the writ signing key holds the authority.** Key custody is the
  operator's responsibility.
- **Policy is only as strong as the loaded policy set**, and approval gates are
  only as strong as the humans clearing them.
- **Default cognition is the mock** unless a key is set — "it responded" locally
  may be the deterministic mock, not a real model (`/health` →
  `cognition_live`).
- **Replay verifies the *record* is intact and consistent; it does not
  re-execute tools.** It trusts the observation committed at run time. Tampering
  and drift are caught; a tool that did something unexpected *at execution time*
  is caught only to the extent its postconditions / effect class describe it.

## How to reproduce

```bash
cd thymos
# Enforcement (no secrets):
cargo test -p thymos-runtime --test compiler_rejection --test revocation \
  --test cognition_budget --test redaction --test replay_safety --test quorum

# Live audit tooling (no secrets; ROOT-only trail under the mock):
THYMOS_LEDGER_PATH=/tmp/thymos-audit-demo.db ./target/debug/thymos-server &
./target/debug/thymos run "Map the repo and summarize the runtime" --follow
./target/debug/thymos runs ls                 # copy the full run id
./target/debug/thymos audit <run-id>

# Commit-bearing trail (needs a key):
ANTHROPIC_API_KEY=sk-ant-… ./target/debug/thymos-server &
ANTHROPIC_API_KEY=sk-ant-… ./target/debug/thymos run "…" --follow
./target/debug/thymos audit <run-id>
```
