# Project status — what is real, what is gated, what is not done

This document exists so nobody — including us — has to guess where the line is
between "implemented and proven" and "wired but unproven" and "not built yet."
It is written to be read adversarially. If something here drifts from the code,
the code wins and this file is a bug.

Last verified against `main` with `cargo test --workspace`: **238 test results
passing, 0 failing, 4 ignored** (the 4 are timing benchmarks, not skipped
features). 237 `#[test]`/`#[tokio::test]` functions across 12 crates.

## Real and proven on every CI run

These are exercised by the default `cargo test --workspace`, with no external
services or API keys:

- **Hash-chained ledger (SQLite).** `id = blake3(canonical_json(payload))`,
  per-trajectory chains, `verify_integrity` recomputes every digest. Tampering,
  root-relabeling, and cross-trajectory id collisions are regression-tested.
- **Deterministic replay.** Folding committed deltas reproduces the world;
  replay *rejects* compiler-version drift, policy-set drift, and unsigned commits
  when signatures are required. Benchmarked at ~84k entries/sec on 1001 entries.
- **Governance, actually enforced in the compiler** (not just described): writ
  tool-scopes, effect ceiling (write / external / irreversible with
  parent-ceiling delegation checks), budget projected from the ledger, time
  windows, revocation with one-level cascade. Each has a dedicated test file
  (`revocation`, `cognition_budget`, `quorum`, `compensation`(+gate,
  +cross-trajectory), `idempotency`, `redaction`, `replay_safety`,
  `compiler_rejection`, `json_policy_e2e`).
- **The agent loop**, end to end, against the deterministic `MockCognition`
  (`submit → compile → govern → execute → ledger`).
- **HTTP server surface**: `/runs`, `/routed-submit`, `/routing-outcomes`, auth
  on control-plane endpoints, tenant scoping — e2e tested.
- **WisePick routing-evidence integration**: forward path + data-sovereignty
  (no intent args / tool output / tenant / writ leak into feedback records).

## Real, but gated (needs external resources, not run in CI)

These are implemented and compile in CI, but cannot run on a hermetic runner.
Each skips cleanly (prints `SKIP`, passes) when its resource is absent, so it
never produces a false failure — and proves the real path when you supply it.

| Capability | Proof | How to run |
|---|---|---|
| **Live LLM cognition** (Anthropic/OpenAI adapters, real HTTP) | `crates/thymos-runtime/tests/live_provider.rs` — drives a real model through the full loop and asserts a real commit mutated the world | `ANTHROPIC_API_KEY=… cargo test -p thymos-runtime --test live_provider -- --ignored --nocapture` |
| **Postgres ledger backend** | `crates/thymos-ledger/tests/postgres_integration.rs` — append → read-back → `verify_integrity` against a real DB. Compile-guarded in CI so it can't bit-rot | `THYMOS_TEST_POSTGRES_URL=… cargo test -p thymos-ledger --features postgres --test postgres_integration -- --ignored` |

## Honest caveats

- **Default cognition is the mock.** If no `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`
  / `THYMOS_DEFAULT_PROVIDER` is set, the server answers runs (that omit their
  own `cognition` block) with the deterministic mock — *not* a real model. The
  server now logs a `WARNING` at startup in that case and `/health` reports
  `cognition_live: false`. Set a key to make it live; the provider is then
  auto-detected.
- **Postgres is now a selectable HTTP runtime backend (feature-gated).** The
  runtime/ledger trait refactor has landed end to end: `thymos-ledger` defines a
  `LedgerStore` trait; `thymos-runtime`'s `Runtime`/`Run` are generic over it
  (`Runtime<L: LedgerStore = Ledger>`); and a `BlockingPostgresLedger` facade
  adapts the async Postgres backend to the synchronous trait by driving futures
  on a dedicated runtime thread (so it is safe to call from the server's async
  handlers — `block_on` would panic there). The server selects the backend at
  startup: with the `postgres` feature built and `THYMOS_POSTGRES_URL` set, it
  runs `Runtime<Box<dyn LedgerStore>>` over Postgres; otherwise it uses SQLite.
  The **default build remains SQLite** (no Postgres dependency compiled). Proof
  is gated on a live database (`THYMOS_TEST_POSTGRES_URL`): one test asserts the
  facade yields a byte-identical content-addressed chain to SQLite from the same
  inputs, another asserts the sync facade is safe to call from inside a tokio
  runtime. These run in the Postgres CI job once DB secrets exist; they skip
  cleanly (print `SKIP`, pass) otherwise.
- **`thymos-worker` is intentionally a thin binary.** It is the process-isolation
  boundary for sandboxed tool execution; the substance lives in
  `thymos_tools::worker_entrypoint` (kept in the library so it is unit-tested and
  can also run in-process). Thinness is the design, not a stub.

## Release status

- Version in `Cargo.toml`: see `thymos/Cargo.toml`. Tagging a `vX.Y.Z` triggers
  `.github/workflows/release.yml`, which builds binaries (linux/macOS/windows),
  pushes GHCR images, and publishes the GitHub Release automatically.
- Releases are tag-driven only; `workflow_dispatch` on a branch will not create a
  release (the publish job is gated on a `refs/tags/v*` ref).
