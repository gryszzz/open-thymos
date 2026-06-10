# OpenThymos Desktop — UX Philosophy

This is the canonical guide for the desktop app's experience. Every desktop PR
is measured against it.

## The one principle

**Standard AI chat experience first. OpenThymos capabilities second.**

It should feel immediately familiar to anyone who has used ChatGPT, Claude,
Cursor, LM Studio, or Open WebUI. We do **not** reinvent the chat experience — we
add governance, replay, and auditability *on top of* a familiar one.

> A premium AI desktop app that happens to have governance — **not** a governance
> system that happens to have a chat box.

The user chats normally. OpenThymos quietly provides skills, tools, providers,
grants, replay, and audit **when needed** — never in the user's face by default.
A user should understand the interface within seconds.

## Default layout

```
┌───────────────┬───────────────────────────────────────────────┐
│  + New chat   │   active model · provider · skills (quiet)     │
│  ⌕ Search     ├───────────────────────────────────────────────┤
│  Chat history │                                                │
│   • …         │            conversation                        │
│   • …         │                                                │
│               ├───────────────────────────────────────────────┤
│  Providers    │  [ + ] message…                       [ Send ] │
│  Skills       │                                                │
│  Tools        │  optional: provider · skill · model (light)    │
│  Audit        │                                                │
│  ⚙ Settings   │                                                │
└───────────────┴───────────────────────────────────────────────┘
```

- **Chat is always the primary screen.** It opens to chat.
- **Sidebar:** New Chat · Search · Chat history. Secondary nav (Providers,
  Skills, Tools, Audit, Settings) lives below, lightweight.
- **Composer at the bottom:** prompt · attachments · send. Optional
  provider/skill/model selectors are *light*, not overwhelming.

## How each capability should feel

- **Skills** — like GPTs / Claude styles / agent profiles. Enable one, several,
  or none without friction. (Multi-skill is shipped.)
- **Tools** — capabilities the assistant has. Users never see schemas or
  internals unless they enter **Advanced Mode**.
- **Grants** — clean and modern. Plain language, not runtime jargon:
  > **OpenThymos wants permission to use the File System tool.** `Allow` `Deny`
  with an *Advanced details* expander, not a wall of writ/policy text.
- **Replay** — "**View what happened**", not "inspect internal runtime
  artifacts." Advanced users can expand into commits/writs.

## Visual design

Prioritize: whitespace · readable typography · smooth animation · modern cards ·
clear icons · clean navigation. Avoid: clutter · developer-only terminology ·
excessive JSON · overwhelming controls · exposing internals by default.

## Advanced Mode

A single toggle (Settings / header). **Off by default.** When off, normal users
**never** see raw runtime data, policies, writs, commits, debug traces, or
low-level tooling. When on, those reveal:

- raw ledger entry data in the Mind inspector and Audit,
- the custom-tool schema editor and raw skill/policy JSON,
- writ / commit / proposal internals, debug/perf info,
- experimental / unstable controls (also gated to Nightly).

Implementation: a `body.advanced` class toggled + persisted; anything
developer-facing is tagged `.adv-only` (hidden unless Advanced). New
internals-exposing UI must be `.adv-only` by default.

## Stable vs Nightly

- **Stable** surfaces: multi-skill selection, basic skill/tool builders,
  provider/model selection, grant cards, replay viewer, the Mind graph.
- **Nightly / Advanced**: raw JSON editing, experimental automation, debug
  traces, low-level runtime internals.

## The test for any desktop change

1. Does chat stay the primary, familiar experience?
2. Is the new power *quiet* until needed?
3. Would a non-engineer understand it in seconds?
4. Is anything that exposes internals behind Advanced Mode?

If a change makes the app feel like a governance console, it's wrong — governance
is the substrate, not the surface.
