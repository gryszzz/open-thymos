---
layout: default
title: Package Distribution
eyebrow: Release protocol
subtitle: OpenThymos ships as release binaries and GitHub Packages container images.
permalink: /package-distribution/
---

# Package Distribution

OpenThymos is distributed through two release artifacts:

- tagged GitHub Releases containing platform binaries
- GitHub Packages container images for the runtime server and CLI

The package channel is operational infrastructure. It must be reproducible from
source, traceable to a Git commit, and safe to consume from automation.

## Package Names

The release workflow publishes two GHCR image names:

```text
ghcr.io/gryszzz/openthymos-runtime
ghcr.io/gryszzz/thymos-server
```

`openthymos-runtime` is the canonical package name. `thymos-server` remains a
compatibility alias for existing users and scripts.

## Publication Triggers

Packages are published by `.github/workflows/release.yml`.

The workflow runs on:

- semver tags matching `v*.*.*`
- manual `workflow_dispatch` invocations

Tagged releases also publish binary archives through GitHub Releases.
Manual dispatches publish branch and SHA-tagged package images without creating
a GitHub Release.

## Tags

Container tags are produced from the Git reference:

| Tag form | Meaning |
| --- | --- |
| `v0.1.0` | Exact release tag |
| `0.1.0` | Semver release tag |
| `0.1` | Semver minor channel |
| `latest` | Latest semver release tag |
| `main` | Manual or branch package build |
| `sha-<revision>` | Immutable source revision tag |

Automation should prefer exact semver or SHA tags. `latest` is for interactive
evaluation only.

## Pull And Run

```bash
docker pull ghcr.io/gryszzz/openthymos-runtime:<tag>

docker run --rm \
  -p 3001:3001 \
  -v "$PWD/.thymos:/data" \
  ghcr.io/gryszzz/openthymos-runtime:<tag>
```

The image starts `thymos-server` by default and stores runtime state at
`/data/thymos-runs.db`.

## Local Verification

Before publishing a package, run the same checks expected by CI:

```bash
npm run verify
cd thymos
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

Then build the container locally:

```bash
docker build -t openthymos-runtime:local thymos
docker run --rm -p 3001:3001 openthymos-runtime:local
```

## Package Invariants

A published package must satisfy these invariants:

- the OCI `source` label points to the OpenThymos repository
- the OCI `revision` label matches the Git commit that produced the image
- the package contains `thymos-server` and `thymos`
- `/health` succeeds after the container starts
- package publication uses GitHub's package-scoped token path, not a local
  developer credential
- package tags are append-only for semver releases

## Separate Agent Repositories

External agent packages, including a future Kaspa AI agent repository, should
publish under their own repository and package identity.

Minimum requirements for a separate repository:

- its own `Dockerfile`
- its own `.github/workflows/release.yml`
- `permissions: packages: write`
- a package name that does not collide with `openthymos-runtime`
- explicit documentation of which OpenThymos runtime version it targets
- no embedded private keys, RPC credentials, seeds, wallet material, or API
  tokens in the image

Suggested GHCR package pattern:

```text
ghcr.io/<owner>/<kaspa-agent-repo>
```

The package should treat Kaspa network access, signing authority, wallet
operations, and RPC endpoints as runtime configuration. They must not be baked
into the artifact.

## Release Discipline

Publishing a package is a protocol event. A maintainer should be able to answer:

- which source revision produced the package
- which compiler/runtime versions were included
- which tests passed before publication
- which image digest was deployed
- whether the package is a release, branch build, or SHA build

If those questions cannot be answered, the package is not release-grade.
