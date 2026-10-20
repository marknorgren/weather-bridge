# ADR-0009: Recover only recorded temporary monitoring pauses

## Status

Accepted for implementation, 2026-10-20. Amends the recovery policy in
[ADR-0002](adr-0002-cloudfront-origin-and-cost-guard.md). Production deployment
and live recovery validation require separate authorization.

## Context

On October 19, a regional Lambda `ListFunctions` request returned
`InternalFailure` after SDK retries. The account-wide usage guard stopped app
concurrency and disabled CloudFront. A complete retry succeeded about a minute
later, below every limit. Scheduled checks could not resume the demo, so the
temporary monitoring fault caused a persistent public outage.

Shutdown protects cost while usage is unknown. Unconditional automatic resume
would undo manual or budget stops. The previous handler did not persist stop
ownership and reduced every SNS message to the same stop action.

## Decision

- Preserve immediate shutdown on incomplete usage evidence.
- Store a versioned stop-state record in one private S3 object in the existing
  artifact bucket. The existing reserved concurrency of one serializes writers.
  Scope object permissions to that key and listing to its exact prefix.
- A recognized temporary usage-read error can pause a running service or reset
  an existing monitoring pause. A successful later check cannot convert a
  locked stop into a pause.
- Require two consecutive complete native scheduled checks below every usage
  threshold. Reject duplicate, early, expired, and foreign checks. A missed
  schedule resets recovery evidence. Verify the budget and both control alarms
  immediately before restoring the configured concurrency and CloudFront.
- Manual, budget, threshold, permission, unexpected-error, unknown-state, and
  interrupted-resume stops require explicit operator resume.
- Persist stop intent before service changes. Persist a locked resume intent
  before enabling either service. State-read or state-write failures cannot
  authorize automatic resume.
- Preserve recent SNS message IDs across resume. Delayed unknown or older
  signals remain conservative locked stops. The existing topic policy and
  Lambda invocation permission remain the signal trust boundary.
- Report handled temporary read failures without raising a Lambda error.
  Unexpected crashes and failed scheduler delivery retain their shutdown
  alarms. Log bounded structured context rather than raw SDK error messages.
- Publish notices on a separate owner topic. An optional operator-selected
  email subscription requires confirmation. Notification failures are logged;
  they do not alter cost-control state.
- Record a manual stop before deployment. Allow the deployment driver to use
  at most twelve 30-second waits for control-alarm initialization, then fail closed.

## Alternatives considered

Keeping manual recovery and adding an owner alert is smaller, but each temporary
read error would still require an operator. Unconditional resume after a
successful check loses manual and budget stop ownership. Increasing SDK retries
reduces some failures but does not repair persistent stop semantics.

Parameter Store was considered for runtime state. The existing private S3
bucket avoids another storage service and provides documented
[read-after-write consistency](https://docs.aws.amazon.com/AmazonS3/latest/userguide/Welcome.html)
for overwrites. CloudFormation does not manage the mutable object value, so a
stack update cannot reset recovery ownership.

## Consequences and validation

Monitoring faults still interrupt service while evidence is unavailable.
Recovery requires two scheduled checks and CloudFront propagation. Other stops
remain operator actions. Billing and CloudWatch publication delays still limit
the cost estimate; the configured thresholds do not change.

Tests must prove pause and recovery, hard-stop precedence, event duplication
and timing, missing or malformed state, partial scans, failed writes, interrupted
resume, failed service enable, notification failure, scoped permissions, and
deployment ordering. Local tests use fake AWS services. Before production
acceptance, validate actual role permissions, confirmed owner delivery, staged
shutdown/recovery, and public REST/MCP behavior under separate deployment
authorization.
