# ADR-0002: Front the Lambda demo with CloudFront and guard its cost

## Status

Accepted, 2026-09-30. Amends [ADR-0001](adr-0001-public-demo-hosting.md). Provider documentation and prices were checked on this date. Deployed with an active Free subscription; live REST/MCP, direct-origin rejection, cache headers and SNS shutdown/recovery were verified.

## Context

A design and security review found three infrastructure problems in the ADR-0001 deployment:

- **Availability (F1).** The Function URL had reserved concurrency 1. A Lambda instance handles one request at a time, so one roughly 6-second uncached report made `/healthz` and every other request return 429. The 4.35 MB Scalar bundle and every page load also went through the same slot.
- **Usage guard (F2).** The guard added 10 seconds of Init to every invocation. The compute threshold tripped near 128,000 requests, so a client looping `/healthz` could stop the demo for the month in about 14 hours.
- **Rollback (F6).** Artifacts expired after 7 days, so CloudFormation could not roll back a failed update made more than 7 days after the previous deploy.

The owner's budget is $0, or at most $1–2 per month. AWS WAF on pay-as-you-go (about $5 per web ACL per month), API Gateway, provisioned concurrency, NAT, and paid CloudWatch features are excluded.

## Decision

1. **Put a CloudFront distribution in front of the Function URL.**
   - Pages, `/openapi.json`, and `/assets/*` are cached with the managed `CachingOptimized` policy, compressed at the edge, and invalidated on each deploy.
   - `/v1/*` is cached only when the app sends `Cache-Control: max-age`, keyed on the full query string and capped at 120 s.
   - `/mcp*` and the diagnostic endpoints (`/healthz`, `/version`, `/metrics`) are never cached. `/mcp*` forwards every viewer header except `Host` and accepts POST.
2. **Protect the origin with a CloudFront origin header, not Origin Access Control.**
   - CloudFront adds `X-Weather-Bridge-Origin-Verify`, and the app rejects requests without its matching value. The Function URL keeps `AuthType NONE`.
   - The readiness exemption is limited to `GET`/`HEAD /healthz` with no query string and a loopback Host of `127.0.0.1:8080`, `localhost:8080`, or `[::1]:8080`. Direct public health requests are rejected; CloudFront health requests carry the origin header normally.
   - OAC requires `AuthType AWS_IAM`. Lambda then requires each POST to carry the SHA-256 of its body in `x-amz-content-sha256` ([AWS documentation](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/private-content-restricting-access-to-lambda.html)). MCP clients do not send it. CloudFront Functions cannot read bodies, and Lambda@Edge has no free allowance.
3. **Make reserved concurrency a parameter** (default 3, range 1–10), restored only by the guard.
4. **Estimate compute from cold starts, not invocations.**
   - A CloudWatch Logs metric filter counts Lambda's `INIT_START` lines for the demo. Each start is charged the 10-second Init limit.
   - Lambda has no free Init metric, and environment lifetime is not guaranteed.
   - The guard falls back to charging every invocation a cold start when the filter is missing or log delivery is incomplete. Only complete hours ending before `now − 15 minutes` use measured starts; the recent 15–75 minute window and unmatched invocations always use the conservative fallback.
5. **Cap CloudFront cost in the guard.**
   - The guard reads `AWS/CloudFront` `Requests` and `BytesDownloaded` for every distribution in the account.
   - At 80% of the always-free 10M requests or 1 TB, it stops the demo and disables this distribution.
   - The cost budget backstop covers CloudFront as well as Lambda.
6. **Offer the CloudFront flat-rate Free plan as opt-in.**
   - The plan is $0 with no overage charges and includes WAF IP rate limiting. CloudFormation can manage it with `AWS::PricingPlanManager::Subscription` ([flat-rate plans](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/flat-rate-pricing-plan.html)).
   - It is not the default because Free-account-plan accounts are ineligible.
   - The plan allows only managed cache policies, so `/v1` is not cached. The managed origin-`Cache-Control` policies key on `Host`, which a Function URL origin cannot accept.
   - Its required web ACL is billed if the subscription lapses.
7. **Keep current artifacts.** Remove the 7-day expiry, and replace artifact objects create-before-destroy.

All new AWS resources live in the CloudFormation template, so Terraform still owns only the two stacks and two artifact objects.

## Alternatives considered

| Option | Why not chosen |
| --- | --- |
| OAC with `AWS_IAM` Function URL | Breaks MCP POST without client-side body hashing. |
| OAC plus Lambda@Edge to hash bodies | Lambda@Edge has no free tier and is billed even on flat-rate plans. It adds a replicated function and is outside the guard. |
| AWS WAF rate limiting on pay-as-you-go | About $5/month per web ACL plus $1/rule. Exceeds the budget. |
| API Gateway throttling | Excluded by the budget constraint and adds a second request meter. |
| Smaller flat per-invocation Init allowance | Underestimates real cold starts or still trips early. The metric-filter count is both cheaper and closer. |
| Lifecycle expiry of 90 days | Still breaks rollback after 90 idle days. Terraform-managed deletion has no such window. |

## Consequences

- Cached pages, assets, and repeated reports no longer occupy Lambda, and one slow report no longer blocks `/healthz` (with concurrency 3 or more).
- The public URL becomes the CloudFront domain. The app enforces `WEATHER_BRIDGE_ORIGIN_VERIFY` and rejects direct Function URL requests, including public `/healthz` requests. Rejected requests still invoke Lambda briefly and count toward the guard's request threshold.
- REST caching depends on the app's `Cache-Control` headers. Their lifetime must stay within the report's remaining freshness budget. The Free plan serves REST uncached regardless of those headers.
- A request flood through CloudFront can still stop the demo for the month on the default plan. The guard caps cost; it does not keep the demo available. Only the flat-rate plan adds per-IP rate limiting at no cost.
- On the default plan, CloudFront charges during the guard's reaction window of about 20 minutes are not bounded against a very large flood. The Free plan removes that exposure.
- Higher concurrency raises the aggregate NWS request rate. Each instance keeps its own upstream limiter.

## Validation

Before sharing the CloudFront URL, verify the following through it:

- REST and MCP behavior;
- direct Function URL rejection;
- `Cache-Control` and `x-cache` headers;
- compressed asset size;
- stop and resume with CloudFront disable/enable;
- the guard's `demoColdStarts` evidence after cold starts.

Record a week of CloudFront and Lambda usage against the allowances.

## Deployment evidence

The initial Free enrollment rejected the distribution configuration. A retry with global edge routing, the default 30-second origin timeout and no custom error-cache overrides enrolled. AWS returned `planTier: FREE` and `status: ACTIVE`. We did not find which setting caused the first rejection, so the Free profile keeps these defaults. On this profile CloudFront does not cache REST responses, but the app still sets their headers from source freshness.

The real SNS shutdown signal set app concurrency to zero and disabled the distribution. After propagation its public hostname stopped resolving. Guarded resume restored concurrency three, enabled the distribution and the health check returned HTTP 200. Local regression checks passed 29 Rust tests and 30 usage-guard tests; the live NWS HTTP/MCP smoke passed. CloudFormation masks the origin parameter, producing a known one-parameter Terraform plan difference; see the operations guide.
