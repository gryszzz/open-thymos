# Design: Enterprise, Machine-Programmability & Monetization

Status: **Draft / strategy** · Scope: how OpenThymos becomes machine-drivable at
enterprise scale and how that is monetized — **without** weakening the core
invariant or the no-phone-home value. Nothing here lets cognition assert
authority; every addition is a control-plane or observability surface around the
existing `Intent → Proposal → Commit` pipeline.

---

## 0. The unfair advantage (what we sell that others can't)

Every LLM agent product bills **per token** and asks you to *trust* what the
agent did. OpenThymos can do the one thing they structurally cannot:

> **Bill per *governed action*, and let the customer audit every charge against
> an append-only, replayable ledger.**

A commit is a content-addressed, hash-chained, optionally-signed record of *one
authorized effect*. That makes a governed action a **countable, provable,
non-repudiable unit** — a billing primitive *and* a compliance artifact at once.
"You were charged for exactly these 1,204 authorized actions; here is the
cryptographic proof of each, under whose writ, and which policy permitted it."
No token-metered competitor can offer provable billing. This is the wedge.

---

## 1. What already exists (the enterprise bones)

Grounded in the code — we are closer than it looks:

| Capability | Where it lives today |
|---|---|
| **API-key gateway + usage tracking** | `middleware::ApiGateway`, `GET /usage` (`usage_stats()`), `thymos-gateway.db` |
| **Multi-tenancy** | `auth::JwtClaims.tenant_id`, tenant-scoped writs, per-tenant concurrency caps |
| **RBAC primitive** | `JwtClaims.roles` (admin-gated control-plane endpoints) |
| **Bearer-token auth** | HMAC-SHA256 JWT middleware |
| **Durable, distributable ledger** | SQLite default; **Postgres backend** selectable on the HTTP runtime |
| **Signed tool supply chain** | `thymos-marketplace` (signed manifests) |
| **Operator-owned telemetry** | OTLP to the operator's endpoint (no phone-home) |
| **Commercial licensing intent** | `COMMERCIAL-LICENSE.md`, `GOVERNANCE.md` |

So the monetization substrate (auth, tenancy, gateway, usage, durable ledger) is
**already present**. The work is to formalize it into products.

---

## 2. Machine-programmability surface (drive it from anything)

"More machine programming" = make every capability reachable, typed, and
automatable by other systems — not just humans in the app.

1. **OpenAPI 3.1 spec** (`docs/openapi.yaml`, shipped alongside this RFC) — the
   contract. Generates typed SDKs in any language, powers API explorers, and is
   the enterprise integration handshake. *This is the single highest-leverage
   machine-programmability artifact and costs no runtime change.*
2. **Webhooks / event egress** — *operator-configured* (never phone-home;
   opt-in, off by default, documented — per the core value) push of governance
   events (`commit`, `rejection`, `approval_required`, `run_finished`) to a
   customer endpoint (SIEM, Slack, PagerDuty, ServiceNow). Turns the runtime
   into a node in an enterprise event mesh.
3. **External approval gateway** — a suspended `Irreversible` proposal calls out
   to the customer's approval system and resumes on signed callback
   (`POST /runs/:id/approvals/:channel`). Human-in-the-loop at enterprise scale,
   recorded on the ledger (who approved, when).
4. **Idempotency keys** on `POST /runs` — machine-safe retries (an at-least-once
   caller can't double-spawn). The commit layer is already idempotent per
   proposal id; extend that to run creation.
5. **Stable, versioned API** + the existing pipe-friendly CLI and Rust SDK
   (`thymos-client`) round out three integration tiers (HTTP, CLI, in-process).

---

## 3. Enterprise edition (what regulated buyers need)

These are the open-core line items — each maps to a real compliance/ops need:

- **SSO / OIDC / SAML** — replace (or augment) the HMAC JWT with enterprise IdP
  federation; map IdP groups → `roles` → writ ceilings.
- **RBAC enforcement** — roles already in the token; enforce role→capability
  (who may approve `Irreversible`, who may mint writs, who may revoke).
- **Provable compliance reports** — signed, exportable audit packs per
  tenant/time-range (commits, rejections, approvals, the policy decision per
  action, replay verdict). The auditor's artifact, on demand.
- **Retention & legal hold** — ledger retention windows, immutable export to
  object storage, e-discovery query API.
- **HA Postgres ledger** — multi-replica, backup/restore, the distributed
  execution ledger (Phase III) productized with an SLA.
- **Data-residency / air-gap** — fully self-hosted, no egress (already a value);
  region-pinned ledgers for the managed offering.
- **Metering API** (§4) — per-tenant governed-action counts for chargeback.

---

## 4. The metering keystone (build this first to unlock revenue)

A per-tenant **metering API** is the smallest change that turns the existing
gateway + ledger into a billable product:

- Aggregate, per tenant + period: `runs`, `commits` (governed actions),
  `rejections`, `approvals`, `tokens`, `tool_calls`, by effect class.
- Source of truth is the **ledger** (already counts these), so every metered
  number is **auditable** — the customer can replay and reconcile their bill.
- Expose `GET /metering?tenant=&from=&to=` (admin/tenant-scoped) + a signed
  export for billing pipelines (Stripe usage records, etc.).
- This is provable usage-based billing — the §0 wedge made concrete.

---

## 5. Monetization model (open-core, three lines)

1. **OSS core (Apache-2.0)** — the runtime, governance, ledger, replay, CLI,
   single-tenant self-host. The credibility + adoption engine. *Never crippled.*
2. **Enterprise edition (commercial license)** — §3 features (SSO, RBAC
   enforcement, compliance reports, retention/legal-hold, HA Postgres, support
   SLA). Sold to regulated industries (finance, healthcare, gov) that *must*
   prove what their agents did. `COMMERCIAL-LICENSE.md` already anticipates this.
3. **Managed Cloud** — hosted governed runtime, **metered per governed action**
   (§4) and/or per operator seat (the web operator console). The meter is the
   gateway + metering API; the proof is the ledger.

Additional streams: **marketplace take-rate** on paid signed tools/skills;
**provable-compliance reports** as a per-seat or per-export add-on.

**Guardrails (non-negotiable):** monetization must never add phone-home or
silent egress; usage signal stays operator-owned and auditable; the OSS core
stays genuinely useful (no "open-core bait"). Candor is the brand.

---

## 6. Real-vs-needed (honest)

| Item | Status |
|---|---|
| API gateway, usage stats, tenancy, roles, JWT | **exists** |
| Postgres ledger backend | **exists** (selectable) |
| OpenAPI spec | **new** — `docs/openapi.yaml` (ships with this RFC) |
| Metering API (`/metering`) | **needs build** (small; aggregates the ledger) |
| Webhooks / event egress | **needs build** (operator-configured, opt-in) |
| Idempotency keys on `POST /runs` | **needs build** (small) |
| SSO/OIDC, RBAC enforcement, compliance export, retention | **needs build** (enterprise edition) |

## 7. Recommended build order

1. **OpenAPI spec** (done here) → unblocks SDKs + enterprise integration today.
2. **Metering API** → unlocks usage-based billing (smallest path to revenue).
3. **Webhooks** → enterprise event-mesh integration.
4. **Idempotency keys** → machine-safe automation.
5. **SSO + RBAC enforcement + compliance export** → the enterprise edition.

Each is independently shippable and strengthens (never blurs) the authority
boundary. Items 2–7 each get their own RFC before code.
