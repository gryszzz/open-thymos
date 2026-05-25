---
layout: default
title: Diagrams
eyebrow: Protocol diagrams
subtitle: Blueprint-style diagrams for OpenThymos runtime semantics.
permalink: /diagrams/
---

# Diagrams

The diagrams below use Mermaid with a dark blueprint theme. They are intended
as protocol diagrams, not product illustrations.

## Intent To Proposal To Commit

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  A[Intent] --> B[Compiler]
  B --> C{Policy}
  C -->|permit| D[Proposal]
  C -->|deny| E[Rejection]
  C -->|approval| F[Pending Approval]
  D --> G[Tool]
  G --> H[Observation]
  H --> I[Commit]
  I --> J[Ledger]
```

## Execution Ledger

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  R[Root seq 0] --> C1[Commit seq 1]
  C1 --> P[PendingApproval seq 2]
  P --> C2[Commit seq 3]
  C2 --> X[Rejection seq 4]
  X --> C3[Commit seq 5]
```

## Runtime Folding

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  W0[World 0] --> D1[Delta 1]
  D1 --> W1[World 1]
  W1 --> D2[Delta 2]
  D2 --> W2[World 2]
  W2 --> D3[Delta 3]
  D3 --> W3[World Head]
```

## Replay Engine

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart TB
  A[Load Entries] --> B[Hash Check]
  B --> C[Parent Check]
  C --> D[Sequence Check]
  D --> E[Fold Commits]
  E --> F[World Hash]
  F --> G[Replay Report]
```

## Provider Abstraction

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart TB
  P1[Anthropic] --> C[Cognition Trait]
  P2[OpenAI] --> C
  P3[Local] --> C
  P4[Hugging Face] --> C
  P5[Mock] --> C
  C --> I[Intent]
  I --> R[Runtime]
```

## Policy Validation Flow

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  I[Intent] --> W[Writ]
  W --> B[Budget]
  B --> T[Tool Scope]
  T --> S[Schema]
  S --> P[Policy Rules]
  P -->|permit| OK[Stage]
  P -->|deny| NO[Reject]
  P -->|approval| HOLD[Suspend]
```

## Capability Writ Lifecycle

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  A[Issue] --> B[Sign]
  B --> C[Verify]
  C --> D[Bind Intent]
  D --> E[Compile]
  E --> F[Debit]
  F --> G[Commit]
  G --> H[Delegate Child]
```

## Agent Execution DAG

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart TB
  R[Root Trajectory] --> A[Agent A]
  A --> B[Child Agent B]
  A --> C[Child Agent C]
  B --> D[Commit B1]
  C --> E[Commit C1]
  D --> F[Parent Reconcile]
  E --> F
```

## Multi-Agent Coordination

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart LR
  W0[Parent Writ] --> W1[Child Writ A]
  W0 --> W2[Child Writ B]
  W1 --> T1[Trajectory A]
  W2 --> T2[Trajectory B]
  T1 --> L[Ledger]
  T2 --> L
  L --> R[Replay DAG]
```

## Runtime State Projection

```mermaid
%%{init: {"theme": "base", "themeVariables": {"background": "#05070b", "primaryColor": "#07111f", "primaryTextColor": "#d6f7ff", "primaryBorderColor": "#38d5ff", "lineColor": "#7dd3fc", "secondaryColor": "#0b1f33", "tertiaryColor": "#020617", "fontFamily": "IBM Plex Mono, ui-monospace, monospace"}}}%%
flowchart TB
  L[Ledger Head] --> F[Fold]
  F --> W[World Projection]
  W --> API[HTTP API]
  W --> CLI[CLI]
  W --> UI[Console]
  W --> AUD[Audit]
```
