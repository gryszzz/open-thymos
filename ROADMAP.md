# OpenThymos Roadmap

This roadmap describes protocol evolution, not product packaging. Each phase
extends the runtime substrate while preserving the core rule: cognition
proposes, the runtime governs, and the ledger records the result.

## Phase I - Deterministic Runtime

### Architectural Goals

- formalize the Intent -> Proposal -> Commit cycle
- make the ledger the source of execution truth
- keep cognition outside the authority boundary
- define world state as a fold over committed deltas
- make policy traces first-class proposal data

### Runtime Capabilities

- signed capability writs
- typed tool contracts
- deterministic proposal compilation
- pending approvals as ledger entries
- local replay verification
- provider adapters that emit intents only

### Protocol Evolution

- stabilize core type schemas
- document content-addressed identity rules
- pin compiler version data on commits
- define rejection, approval, delegation, and branch entry semantics

### Execution Guarantees

- no tool execution without staged proposal
- no projected state mutation outside commit folding
- no provider-specific execution authority
- replay verifies sequence continuity and parent linkage

### Scaling Implications

Phase I is optimized for single-runtime correctness. Storage may be SQLite or
Postgres, but protocol work remains local and inspectable.

## Phase II - Multi-Agent Coordination

### Architectural Goals

- represent delegation as explicit runtime structure
- constrain child agents through child writs
- preserve parent-child trajectory linkage
- expose execution DAGs without weakening replay

### Runtime Capabilities

- signed child writ minting
- delegation keyring support
- child trajectory lifecycle
- DAG traversal for delegated execution
- bounded coordination policies

### Protocol Evolution

- define child trajectory metadata
- formalize delegation entry compatibility
- specify DAG projection from ledger entries
- introduce coordination-level policy rules

### Execution Guarantees

- child writs are strict subsets of parent writs
- tenant boundaries cannot be crossed by delegation
- delegated execution remains replayable through trajectory references
- parent state cannot be mutated by child execution without an explicit commit

### Scaling Implications

The runtime begins to support concurrent work while preserving local authority
and ledger-level lineage.

## Phase III - Distributed Execution Ledger

### Architectural Goals

- separate ledger protocol from storage backend
- support multi-node ledger ingestion
- define conflict and merge semantics for distributed trajectories
- preserve deterministic folding under replication

### Runtime Capabilities

- Postgres-backed production ledger mode
- append verification across nodes
- ledger export and import
- hash-chain audit proofs
- snapshot-assisted replay

### Protocol Evolution

- define ledger segment format
- define replay checkpoints
- specify branch and merge entry semantics
- introduce compatibility rules for ledger transport

### Execution Guarantees

- replicated entries preserve content identity
- replay result is independent of storage backend
- merge points are explicit and content-addressed
- corrupted or reordered entries are rejected

### Scaling Implications

The runtime can move from single-process operation to distributed storage
without changing the execution protocol.

## Phase IV - Runtime Federation

### Architectural Goals

- allow independent OpenThymos runtimes to exchange authority and execution
  records
- define trust roots for federated writs
- preserve replay across organizational boundaries
- make provider choice local while execution semantics remain portable

### Runtime Capabilities

- federated writ verification
- remote trajectory references
- cross-runtime audit queries
- runtime capability discovery
- importable policy bundles

### Protocol Evolution

- define federation identity format
- specify remote ledger reference semantics
- introduce trust-root rotation rules
- define policy bundle compatibility

### Execution Guarantees

- remote authority is explicit and signed
- imported ledger segments remain verifiable
- provider outputs do not cross the effect boundary
- federation cannot bypass local policy

### Scaling Implications

Federation permits cooperation between runtimes without requiring one global
control plane.

## Phase V - Autonomous Governance Layers

### Architectural Goals

- introduce governance agents that propose policy, writ, and runtime changes
- keep governance agents subordinate to protocol rules
- make governance decisions replayable
- encode institutional memory as auditable runtime state

### Runtime Capabilities

- governance proposal queues
- policy simulation against historical ledgers
- writ issuance workflows
- automated invariant checks
- governance audit projections

### Protocol Evolution

- define governance proposal types
- specify approval thresholds and quorum records
- add policy simulation result entries
- define governance replay semantics

### Execution Guarantees

- autonomous governance cannot grant authority outside its writ
- policy changes are committed as inspectable protocol events
- historical replay can identify which policy version governed each proposal
- governance actions remain reversible only when their effect class permits it

### Scaling Implications

The runtime becomes an institutional substrate: capable of proposing changes to
its own operating rules while remaining constrained by recorded authority.
