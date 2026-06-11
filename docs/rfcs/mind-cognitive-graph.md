# OpenThymos RFC

## Title

Mind as a navigable cognitive graph (sessions, concepts, decisions, ledger explorer).

## Status

Accepted

## Summary

Evolve Mind (and the surrounding desktop surfaces) from a per-run
visualization into a navigable cognitive graph over everything the runtime
records: conversations grouped into sessions, a first-class Concept entity
aggregating related records, a Decisions view, a filterable Ledger Explorer,
a relationship graph, and global search. This affects operator surfaces and
adds **new derived entities** (Concept, Decision, Session metadata); it does
**not** change runtime semantics, ledger entry kinds, hashing, replay, or
writs. All new entities are projections over (or annotations alongside) the
existing ledger — never substitutes for it.

## Motivation

Mind and Audit currently render one run at a time, chronologically. As usage
accumulates, knowledge is trapped in an infinite scroll: there is no way to
find "everything that touched governance", to revisit why a decision was
made, or to navigate from a conversation to the commits it caused. The
runtime records all of this; the surfaces don't expose the relationships.

## Current Semantics

- A desktop *chat* is a client-side grouping (localStorage) of messages; each
  message creates one governed run (`POST /runs` with capped conversation
  context). Messages persist `run_id` + `trajectory_id`.
- The ledger records, per trajectory: root, commits, rejections, pending
  approvals, delegations, skill bindings. The execution session adds the live
  narrative (intents, proposals, grants, executions, errors) with timestamps,
  tools, and intent rationales.
- Mind renders one run's events as lifecycle lanes; Audit renders one run's
  narrative + ledger chain. `/audit/entries` supports `run_id`, `kind`,
  `from`/`to` filters. No cross-run aggregation, no tagging, no search.

## Proposed Semantics

Staged so every shipped stage is fully backed by real data.

### Stage 1 — Ledger Explorer + global search (projection only, no new entities)

- **Ledger Explorer** view: all runs' entries in one timeline, grouped by
  day / by run (= conversation message) / by kind, with filters for commits,
  rejections, approvals, delegations, skills. Server: extend `/audit/entries`
  to operate without `run_id` (cross-trajectory page) — read-only projection.
- **Search**: lexical search across run tasks, final answers, session-log
  titles/details, and ledger entry kinds/ids. Server: `GET /search?q=` over
  the run store + session logs. (Semantic search is out of scope until an
  embedding store exists; do not fake it.)

### Stage 2 — Sessions and Decisions (derived views + client annotations)

- **Sessions**: the desktop chat becomes the session unit. Title (exists),
  tags (new, stored client-side per chat), linked runs (exist), and a
  rollup (commits / rejections / approvals counts from the runs). Summaries,
  if added, are model-generated **through a governed run** and stored as a
  message — never silently fabricated.
- **Decisions**: a filtered projection — every approval resolution and every
  rejection, with the intent rationale (already recorded), the proposing
  conversation (run → chat backlink), resulting commits, and a replay link.
  No new ledger kind: a Decision *is* (approval|rejection) ∪ its context.

### Stage 3 — Concepts and the memory graph (new derived entity, optional index)

- **Concept**: a named tag entity with edges (`related_to`, `references`,
  `depends_on`, `created_by`) to sessions, runs, ledger entries, and
  memories (`memory_store` world entries). Stored in a sidecar index (SQLite
  next to the run store), explicitly **derived/annotative**: deleting it
  loses no truth; the ledger remains the single authority. Concept
  assignment is manual first; model-suggested assignment must flow through a
  governed run.
- **Memory graph view**: nodes = concepts, sessions, decisions, memories;
  edges as above; rendered with the existing Mind scene machinery (typed
  nodes, inspector, filters, search dimming).

### Invariants

- No new authority: none of these entities can cause execution; they are
  navigation over the record.
- Replay and ledger hashing are untouched; the sidecar index is rebuildable
  from the ledger + run store + client metadata.
- No model-generated content (summaries, concept suggestions) enters any
  view except as the recorded output of a governed run.

## UI Direction

Mind's left rail becomes a navigator: Active run (today's lanes view) ·
Conversations · Decisions · Ledger Explorer · Concepts · Search. Obsidian /
graph-explorer feel: navigate relationships, not scroll. The current
always-on lifecycle view remains the "Active Thoughts" pane.

## Compatibility

Pure addition. Existing endpoints unchanged (one extended), existing views
remain. Desktop-only metadata (tags) degrades gracefully when absent.

## Open Questions

- Where do session tags live long-term: client localStorage (Stage 2) or the
  sidecar index (Stage 3) so the CLI can see them?
- Should `memory_store` entries gain optional `concept` fields in their args
  schema (governed, recorded) instead of sidecar-only linkage?
- Embedding store for semantic search: local-only model? Which one, and is it
  acceptable under no-phone-home (must be fully local)?
