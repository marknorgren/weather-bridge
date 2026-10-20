# Recover from temporary usage-monitor failures

This living plan uses the exec-plan skill baseline. Tracker: wb-6sr.

## Purpose / Big Picture

A temporary AWS usage-read failure must stop the demo immediately, then permit recovery after two separate successful scheduled checks. Budget, usage-limit, manual, permission, unexpected-error, and unknown stops require explicit operator recovery.

## Scope and Boundaries

Change infra/aws/guard.py, template.json, deploy.py, Terraform inputs, tests, operations docs, and the deployment architecture description. Keep account-wide limits and app interfaces. Add scoped durable state and optional owner notifications. No live resume, resource creation, deployment, PR, or merge. Feature push requires successful fetch/rebase and the complete repository gate. Missing notification address leaves email delivery unconfigured.

## Progress

- [x] 2026-10-20: CloudWatch/CloudTrail established the October 19 incident (04:04 CDT ListFunctions InternalFailure, successful retry about one minute later, persistent stop).
- [x] 2026-10-20: Created codex/guard-recovery from local main 8c749fe. Fetch failed because the managed Git SSH proxy is unavailable.
- [x] 2026-10-20 22:37Z: Captured red-first regression output; final mutation checks reject disabled recovery and recovery eligibility for deliberate stops.
- [x] 2026-10-20 22:37Z: Implemented versioned private S3 state, signal ownership, two-check recovery, scoped IAM, diagnostics, optional owner delivery, and deliberate deployment stop.
- [x] 2026-10-20 22:37Z: Updated operations guide, ADR-0009 and deployment diagram; reviewed state writes, unknown stops, event duplication, interrupted enables, option input, notifications, and deployment ordering.
- [x] 2026-10-20 22:42Z: Final full just check passed with elevated loopback access (146 Rust tests, 29 frontend tests, 32 tooling tests, 103 infrastructure tests, offline HTTP/MCP/CLI smoke). Focused security gate, CloudFormation validation, Terraform formatting, and LikeC4 validation/build passed.
- [x] 2026-10-20: Prepared the reviewed local handoff and Conventional Commit. Fetch/push remain blocked by the managed SSH proxy.

## Surprises & Discoveries

The original SNS handler collapses budget and control alarms into one stop action. A handled monitoring pause must not emit an unexpected Lambda error that becomes a deliberate stop through that backstop. The guard already serializes invocations with reserved concurrency one.

## Decision Log

2026-10-20, Codex: Keep immediate fail-closed shutdown. Record stop ownership in one fixed object in the private artifact bucket. Only recognized temporary usage-read errors from a running service can create a recoverable pause. Unknown or missing state cannot auto-resume. Two distinct scheduled event timestamps, separated by the schedule interval, are required. Preserve deliberate stop signals and bounded duplicate history across resume. Check control alarms and the budget before restoring service. Persist intent before external changes; a partially completed resume requires explicit recovery. Existing concurrency limits and origin protection remain unchanged.

2026-10-20, Codex: Use S3 instead of the initially proposed SSM parameter. S3 documents read-after-write consistency for overwrites, and the private bucket already exists. A prefix-limited list distinguishes absent state from denied access; only explicit resume can authorize running state after an absent record. This also prevents a stack update from resetting mutable runtime state.

2026-10-20, Codex: Compared retaining manual recovery plus owner alerts with automatic recovery limited to recorded monitoring pauses. The user approved the latter. Alerts use a separate owner SNS topic to avoid a shutdown loop. A caller-selected optional email address is deployment configuration, with no subscription created in this task.

## Outcomes & Retrospective

The local implementation and review are complete. The guard preserves immediate shutdown and can recover only a recorded monitoring pause after two separate complete scheduled checks. Tests cover deliberate stops, unknown state, missing checks, failed regions, failed writes, interrupted resume and owner delivery failures. No source dependencies were added. No live resources were changed. Deployment acceptance remains an operator responsibility after separate approval. The existing docs copier drops the separate LikeC4 favicon asset; repairing that unrelated behavior is outside this guard change. Email delivery remains unconfigured until the owner supplies an address and confirms the later SNS subscription.

Review also preserved manual stop when the previous state is absent or malformed, so deployment can record a deliberate stop without first restarting the service. Review clarified that an idempotent direct service stop during a pause cannot record operator intent. Ordinary manual stops must use the guard; emergency direct stops must suspend guard execution until locked state is restored. Review repaired missed-schedule counting, interrupted-resume authority, release-stop ordering, alarm-initialization waits, and unsafe email configuration. Structured usage errors suppress raw SDK exception text while retaining region, operation, code and request ID. [Automation]: persistent stop ownership and explicit recovery evidence make automatic actions observable without undoing operator stops. [Gap: Agentic, deployed role permissions and confirmed owner delivery require separately authorized live acceptance]. Local CodeQL and remote CI were not run; no scanner-closure claim is made.

## Context and Orientation

infra/aws/guard.py reads monthly Lambda usage in every enabled region and CloudFront usage account-wide. Exceptions stop the app and distribution. template.json owns the function, permissions, five-minute EventBridge schedule, budget and alarms. deploy.py owns guarded deployment sequencing; Terraform passes stack inputs. S3 stores a small guard-state record with strong read-after-write consistency. A locked stop needs operator resume; a paused stop can recover after complete checks. The AWS provider failure itself cannot be repaired in this repository.

## Plan of Work

First add handler regressions using fake AWS services and real state persistence boundaries. Show temporary failure followed by two scheduled checks fails on baseline. Then implement state transitions and wire CloudFormation/IAM/scheduler metadata. Update deployment sequencing and optional email delivery. Finally review signal trust, duplicates, crash ordering, hard-stop precedence, missing state, failed enables, and notification failures before committing.

## Concrete Steps

Work in the task worktree. Run these commands with the installed Python 3.14 interpreter first on focused tests, then through the repository gates:

```sh
python3 -m unittest discover -s infra/aws -p test_guard_recovery.py
python3 -m unittest discover -s infra/aws -p 'test_*.py'
just check-security
just check
```

Use PATH with /opt/homebrew/bin first when system Python 3.9 cannot run repository tooling. No tests may call real AWS or Terraform. Preserve the clean default checkout and any unrelated changes.

## Validation and Acceptance

Prove immediate shutdown after a temporary read error; two separate full checks before recovery; duplicate, early, stale and manual checks do not advance recovery; no partial-region recovery; budget/manual/threshold/permission/unknown stops remain locked; delayed duplicate signals cannot clear deliberate stops; unreadable state fails closed; failed resume stops again; notification failures are visible without corrupting stop state. Validate scoped permissions, single writer, schedule metadata, optional notification wiring, and release packaging. Run the required full gate, record blocked commands precisely, and do not push on a red gate.

## Idempotence and Recovery

State writes precede stop and resume side effects. Repeated stops remain safe. A crash during resume leaves a locked state. A missing or malformed record requires explicit resume; never infer recovery eligibility from concurrency zero. Do not remove an unmerged branch. Preserve committed work for review. No remote rollback is needed because no deployment is authorized.

## Artifacts and Notes

Final evidence on 2026-10-20:

- `env PATH="/opt/homebrew/bin:/opt/homebrew/opt/rustup/bin:$PATH" PYTHONDONTWRITEBYTECODE=1 just check`: passed outside the socket-restricted sandbox. The initial sandbox attempt failed at loopback bind, then the elevated complete run passed.
- `env PATH="/opt/homebrew/bin:$PATH" PYTHONDONTWRITEBYTECODE=1 just check-security`: passed with elevated socket access. The final repeat passed at the commit checkpoint.
- `python3 -m unittest discover -s infra/aws -p test_guard_recovery.py`: baseline lacked recovery and accepted an unrecognized SNS signal. Mutating final recovery off, or allowing a locked stop to become paused, makes the corresponding regression fail. A third mutation confirms that manual stop must remain available when previous state is absent.
- `terraform fmt -check infra/aws/terraform`: passed using the installed 1.14.0 binary. No Terraform apply or plan was run.
- `aws cloudformation validate-template --template-body file://.../infra/aws/template.json`: passed through the validation-only API. No resources created or changed.
- LikeC4 1.59.4 (the repository workflow version): `validate docs/architecture` and single-file `build docs/architecture` passed. The build reports a separate favicon SVG asset. The pre-existing docs copier copies only index.html, so that favicon is absent from the combined docs artifact; this does not affect the guard or model validation. No model warnings were reported.
- `just check-commit 'fix(guard): recover temporary monitoring pauses safely'`: passed.
- Git fetch over both the configured remote and HTTPS request was blocked by the configured HTTPS-to-SSH rewrite and absent managed SOCKS proxy. Do not bypass the required transport or claim an up-to-date remote base. Local main was fast-forwarded only to its already-fetched 8c749fe commit. No push is permitted until fetch/rebase/full verification succeeds.

Evidence and review outcomes belong here and in wb-6sr. Do not store account identifiers, credentials, or raw service payloads in tracked files.

## Interfaces and Dependencies

Use the existing Python runtime and boto3 in Lambda. Use the existing private artifact bucket for one guard-state object and add one owner SNS topic, each scoped to this stack. Do not add a database, external dependency, or generic retry framework. The state schema is versioned and rejects unknown modes and malformed fields. Explicit resume remains the operator recovery entrypoint.

## Revision Note

2026-10-20, Codex: Initial execution contract after approved recovery proposal. Implementation, review, local validation and handoff completed; the commit contains this final evidence. No live state changed.
