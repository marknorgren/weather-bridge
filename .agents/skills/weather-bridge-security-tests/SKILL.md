---
name: weather-bridge-security-tests
description: Add or run behavior-focused security regression tests for Weather Bridge release artifacts, deployment arguments, DNS validation, logs, and frontend contracts.
---

Read [Contributing](../../../docs/contributing.md). Run `just check-security`
for a focused pass, then the required `just check` after the final code change
and again after a pre-push rebase. A focused pass does not replace the full gate.

For a new behavior, first show that the proposed test fails before the fix.
Already-safe behavior can use existing passing evidence; label that case.
Choose assertions about the boundary's observable behavior:

- Release paths: traversal and symlink roots, files, and destinations fail;
  outside files stay unchanged. Valid absolute temporary directories still
  package, verify, and extract. Tampered archives and wrong revisions fail.
- Deployment: option prefixes and controls fail before any subprocess call.
  Capture argument arrays for normal profiles, profiles with spaces, partition
  regions, and omitted-profile OIDC. Never execute AWS or Terraform in a test.
- DNS: enforce 63-character labels and 253-character names, including ACM
  underscores. Reject long failing suffixes and foreign destinations. Preserve
  valid boundary values, trailing-dot normalization, and conflict-before-write.
- Build/log output: execute the real build script with CR, LF, terminal
  controls, bad lengths, and non-hex revisions. Check valid and missing revision
  output. Fixture logs must report the actual bound port and cannot forge lines.
- Frontend: retain constant-pattern parity with the shared city fixtures,
  isolated page state, controlled time, cancellation, stale observations, and
  failed alert checks. Keep source lint and generated-artifact freshness checks.

Run `just check-browser` when the page initializer, entry point, or generated
bundle changes. Use bundled Playwright browsers and the owned offline server.
Run `just check-docs` when docs-browser behavior changes. Run `just check-live`
only when NWS fetching or parsing changes; fixture success is not live evidence.

GitHub's extended CodeQL scan remains a separate gate. If running CodeQL
locally, enable local sources as well as the default remote sources and compare
the original alert rules on baseline and candidate source. An extraction failure,
skipped language, or missing result is not a clean scan. Keep scanner evidence
private and report exact commands, results, and unresolved checks.
