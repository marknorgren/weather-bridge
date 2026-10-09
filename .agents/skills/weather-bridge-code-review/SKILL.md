---
name: weather-bridge-code-review
description: Review Weather Bridge changes to release paths, deployment arguments, DNS parsing, diagnostic output, or frontend test execution for security bypasses and regressions.
---

Read [AGENTS.md](../../../AGENTS.md) and the relevant callers and tests. Review
the candidate diff before committing a security fix. Report concrete paths and
observable failures; distinguish an exploitable input from intentional operator
configuration or trusted repository code.

Trace each changed input to its sink and check the shared boundary:

- Release tools accept operator-selected absolute directories, including
  downloaded rollback artifacts. Reject parent traversal and artifact symlinks;
  resolve child paths and check containment before file access. Keep exact ZIP
  member allowlists, digests, revision checks, and verification before AWS access.
- Deployment commands use argument arrays. Check profile and region validation
  before subprocess calls and keep option values attached to their named option.
  A shell-free command still needs protection against option interpretation.
- DNS validation uses bounded ASCII labels and exact ACM/CloudFront suffixes.
  Preserve lowercase hostname rules and normalization of ACM trailing dots.
- Cargo directives must contain only the validated revision or constant
  `unknown`. Fixture readiness output uses the actual socket address. Error
  text must not create new log lines or terminal control sequences.
- Frontend tests import the page initializer. Test generated bundles in the
  browser fixtures. Treat schema patterns as data: compare against reviewed
  constant patterns before using those constants, with both Unicode modes.

Inspect every changed helper's direct callers. Try one alternate malicious
representation and one legitimate input. Check failed paths for partial writes,
AWS/Terraform calls, lost error detail, and changed observation/alert semantics.

Use [weather-bridge-security-tests](../weather-bridge-security-tests/SKILL.md)
for evidence. GitHub CodeQL uses the extended suite with remote and local
sources. Tests passing does not establish scanner closure. Record scanner
version, threat model, analyzed revision, remaining results, and any unavailable
check. Do not dismiss alerts, exclude files, or weaken queries to obtain a pass.
Review does not authorize deployment, a PR, or changes to GitHub settings.
