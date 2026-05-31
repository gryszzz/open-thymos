# Security Policy

## Supported Versions

OpenThymos is currently in active pre-v1 development.

Security fixes are applied to the latest development branch and future releases published through the official releases page.

| Version                 | Supported      |
| ----------------------- | -------------- |
| `v0.1.0`                  | ✅              |
| Latest Release          | ✅              |
| Older Releases          | ⚠️ Best Effort |
| Archived Commits        | ❌              |
| Forks / Modified Builds | ❌              |

Official Releases:

https://github.com/gryszzz/open-thymos/releases

Repository:

https://github.com/gryszzz/open-thymos

## Reporting a Vulnerability

Please **do not open a public issue** for security vulnerabilities.

Preferred reporting channels:

* GitHub Security Advisories:
  https://github.com/gryszzz/open-thymos/security/advisories

* Repository Security Page:
  https://github.com/gryszzz/open-thymos/security

If Security Advisories are unavailable, contact the maintainers through the repository.

## Security Scope

Examples of security-sensitive issues include:

* Governance bypasses
* Policy enforcement failures
* Proposal or Writ authority escalation
* Effect ceiling bypasses
* Replay integrity failures
* Ledger tampering
* Signature verification flaws
* Secret redaction failures
* Budget enforcement bypasses
* Unauthorized tool execution
* Capability routing trust-boundary violations

## Response Process

Maintainers will:

1. Acknowledge receipt of the report.
2. Validate the issue.
3. Assess severity and impact.
4. Develop and test a fix.
5. Coordinate disclosure when appropriate.

## Security Philosophy

OpenThymos is built around governed machine action.

Cognition does not possess direct execution authority.

Machine actions should be:

* Governed
* Auditable
* Replayable
* Constrained
* Explicitly authorized

Vulnerabilities that compromise these guarantees are treated as high priority.

For project updates and releases:

https://github.com/gryszzz/open-thymos/releases
