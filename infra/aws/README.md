# AWS deployment and usage guard

A CloudFront distribution is the public endpoint, in front of one Lambda Function URL.

- CloudFront caches pages, page scripts and the OpenAPI document. On pay-as-you-go it briefly caches REST reports when the app allows it; the Free plan leaves REST uncached. MCP is never cached.
- A startup wrapper runs `weather-bridge serve` on ARM64/AL2023 with the AWS Lambda Web Adapter 1.0.1 layer (version 28).
- The app has 256 MB memory, a 60-second Lambda timeout, no provisioned concurrency, and reserved concurrency 3 (configurable 1–10) while running.

[ADR-0002](../../adrs/adr-0002-cloudfront-origin-and-cost-guard.md) explains the design.

## Deploy

Install AWS CLI, Terraform, Cargo Lambda, and Zig. Build metadata is part of the
release contract: `WEATHER_BRIDGE_BUILD_REVISION` must be the exact lowercase
40-character Git commit ID. Cargo Lambda's `--arm64` option builds the existing
`aarch64-unknown-linux-gnu` target used by the `arm64` AL2023 function.

```sh
REVISION="$(git rev-parse HEAD)"
export WEATHER_BRIDGE_BUILD_REVISION="$REVISION"
cargo lambda build --release --arm64 --output-format zip --locked
python3 scripts/release.py package --compiled-zip target/lambda/weather-bridge/bootstrap.zip --output "target/releases/$REVISION" --revision "$REVISION"
python3 infra/aws/deploy.py --profile your-profile --region us-east-1 --contact https://example.com/weather --release "target/releases/$REVISION" --revision "$REVISION"
```

Options: `--concurrency 1-10` (default 3) sets the reserved concurrency that the guard restores. `--cloudfront-plan FREE` subscribes the distribution to the CloudFront flat-rate Free plan (see [CloudFront pricing plan](#cloudfront-pricing-plan)).

`scripts/release.py package` creates the final `app.zip` (the Rust executable plus
the Lambda startup wrapper), `guard.zip`, and `release-manifest.json` exactly
once. The manifest records the build revision, `aarch64-unknown-linux-gnu`
target, sizes, and SHA-256 digests. Packaging checks that the executable is a
64-bit AArch64 ELF and contains the requested revision. `deploy.py` requires
this release directory and expected revision. It verifies and stages all three
files before its first AWS or Terraform command, then verifies the staged copy
again. A missing manifest, different revision, changed digest, wrong binary
architecture, or wrong wrapper stops without touching AWS.

Release tools accept relative paths and explicit absolute directories, including
temporary downloaded artifacts. Parent traversal and artifact symlinks are
rejected. Fixed-name files must resolve inside their selected release directory.
Choose a directory that other users cannot change while packaging or deploying.
AWS profiles must start with an ASCII letter or digit and contain only letters,
digits, spaces, `_`, `.`, `@`, `:`, `/`, or `-` (at most 128 characters). Region
values must use the AWS region-name form; values are passed as single named
options before any AWS command runs.

To roll back locally, download or retain a previous verified release directory
and pass its manifest revision to `--revision`. Deployment never rebuilds a
rollback revision.

## GitHub release workflow and OIDC bootstrap

[The release workflow](../../.github/workflows/release.yml) builds and stores an
immutable release artifact for every push to `main`, after `just check`, the
minimum-Rust-version check, and the dependency/license audit pass for that exact
commit. Deployment is default-off: the deploy job runs only in
`marknorgren/weather-bridge`, from `refs/heads/main`, when the repository Actions
variable `AWS_DEPLOY_ENABLED` is exactly `true`. It is a
repository variable because the job-level gate is evaluated before the
environment job starts. Pull requests and forks cannot enter the deploy job.

Bootstrap the deployment separately from the workflow:

1. Add `https://token.actions.githubusercontent.com` as an AWS IAM OIDC provider
   with audience `sts.amazonaws.com`, then create a deployment role with only
   the CloudFormation, artifact-bucket, Lambda concurrency/invoke, CloudFront
   invalidation, ACM read, and Terraform discovery/import permissions needed by
   these two fixed stacks. There are no long-lived AWS keys in GitHub.
2. Limit the role trust policy to audience `sts.amazonaws.com` and the exact
   repository subject for `environment:weather-demo`. Inspect the repository's
   OIDC settings with `gh api repos/marknorgren/weather-bridge/actions/oidc/customization/sub`.
   With the default subject template, append `:environment:weather-demo` to
   the returned `sub_claim_prefix`. This repository uses immutable subjects,
   whose prefix includes the owner and repository IDs:
   `repo:OWNER@OWNER-ID/REPO@REPO-ID`. The older name-only prefix does not match.
   If a repository uses a custom subject template, follow that template instead.
   GitHub uses the environment form of the subject when a job names an environment;
   keep the environment's `main` branch restriction below.
3. Create the GitHub environment `weather-demo`. Restrict its deployment branch
   to `main`, add a required reviewer if desired, and disallow administrator
   bypass when the repository plan supports it. The environment branch rule is
   the ref restriction; the IAM subject supplies the repository and environment
   restriction.
4. Configure these non-secret `weather-demo` environment variables: `AWS_ROLE_ARN`,
   `AWS_ACCOUNT_ID`, `AWS_REGION` (normally `us-east-1`), and `NWS_CONTACT`.
   Optional variables are `RESERVED_CONCURRENCY` (1-10), `CLOUDFRONT_PLAN`
   (`PAY_AS_YOU_GO` or `FREE`), and `CUSTOM_DOMAIN`.
5. Run the workflow once with deployment disabled and inspect its checks and
   uploaded manifest. Create the repository Actions variable
   `AWS_DEPLOY_ENABLED=true` only after the role,
   environment branch restriction, account concurrency, CloudFront plan
   eligibility, and target values have been reviewed.

The IAM trust shape follows the current [GitHub OIDC for AWS
guidance](https://docs.github.com/en/actions/how-tos/secure-your-work/security-harden-deployments/oidc-in-aws)
and [AWS IAM guidance for GitHub's OIDC
provider](https://docs.aws.amazon.com/IAM/latest/UserGuide/id_roles_create_for-idp_oidc.html).
GitHub's [OIDC reference](https://docs.github.com/en/actions/reference/security/oidc)
describes immutable subjects and their owner/repository IDs.
Both recommend constraining the `sub` claim; both also recommend environment
protection and deployment-branch rules when the subject names an environment.
The workflow grants `id-token: write` only to the environment-gated deploy job,
pins every third-party action to a full commit, checks the returned AWS account
ID, and obtains credentials only after the GitHub artifact archive and release
manifest have been verified.

For rollback, manually dispatch the workflow on `main` with the numeric run ID
of an earlier successful release workflow. The resolver accepts only an
unexpired artifact from a successful `main` run of this workflow in this
repository, verifies the GitHub artifact archive digest, then verifies its own
manifest and embedded revision. The deploy job never checks out or rebuilds the
old source with its AWS token. GitHub documents the artifact digest in the
[Actions artifact API](https://docs.github.com/en/rest/actions/artifacts) and
the use of environments and branch controls in [deployment environment
guidance](https://docs.github.com/en/actions/reference/workflows-and-actions/deployments-and-environments).

The workflow does not provision the OIDC provider, deployment role, environment,
or variables. Until an operator completes those external prerequisites and
explicitly enables deployment, it only builds and retains release artifacts.
The deployment step uses AWS CLI standard retries with at most ten attempts.
This lets its final guard resume retry throttling while a scheduled check occupies
the guard's single reserved execution slot; the concurrency limit stays at one.

Terraform is the deployment entry point. The driver runs Terraform plan and apply, and uses the AWS CLI for discovery, cache invalidation, and stop and resume.

- Terraform manages two CloudFormation stacks: `weather-bridge-artifacts` and `weather-bridge-demo`.
- IAM changes are limited to roles for these functions.
- Deployment artifacts are encrypted and private. Terraform deletes an old artifact only after the stack stops using it, so CloudFormation can always roll back to the running code.
- Other account resources and budgets are not changed.
- The names are fixed, so deploy one copy per account.

The Terraform module is in [terraform/](terraform/README.md). It manages the stacks and artifact objects. The JSON templates in this directory define every resource inside the stacks. Terraform imports CloudFront-based stacks without recreating them, and each resource has one owner. Pre-CloudFront Function URL stacks are no longer supported by the driver.

The demo starts with concurrency zero. A deployment:

1. Generates an origin-verification value once and keeps it in ignored `target/aws-deployment/origin-verify-secret`.
2. Applies the stacks, then reads the generated CloudFront domain and Function URL host. A second apply configures the exact MCP hosts (CloudFront domain, Function URL host, loopback) and origin (CloudFront).
3. Invalidates `/*` in CloudFront so cached pages and assets match the new build.
4. Calls the usage guard, which enables the configured concurrency and the distribution only below every threshold.

The endpoint and operational metadata are recorded in ignored `target/aws-deployment/state.json`; do not publish that local account state.

The guard has 256 MB memory to load SDK clients for Lambda, CloudWatch Logs, CloudWatch and CloudFront. Its own usage counts toward the account checks. The guard math is tested without AWS access. Boto3 is optional locally; the deployed Python 3.13 runtime includes it:

```sh
python3 -m unittest discover -s infra/aws -p 'test_*.py'
```

This runs guard, release, routing, and DNS helper tests, as CI does.

### Known plan difference

CloudFormation hides the `NoEcho` origin-verification parameter when read. So AWS provider 6.67 shows that parameter as changed on every plan, even when it isn't. Applying it is safe: CloudFormation ignores an identical value. Keep `NoEcho` on and don't add the parameter to `ignore_changes`, or the provider would send the masked value during an unrelated update. To rotate the value, see [Origin protection](#origin-protection).

## Request routing and caching

| Path | CloudFront cache | Notes |
| --- | --- | --- |
| `/mcp*` | Never | All methods. Forwards every viewer header except `Host`, including `Accept`, `Content-Type`, `MCP-Protocol-Version`, `Mcp-Session-Id`, and `Origin`. HTTPS only. |
| `/v1/*` | Pay-as-you-go: at most 120 s when the app allows it. Free plan: never. | Pay-as-you-go uses the full query string as the cache key and default TTL 0. `no-store` is honored. |
| `/healthz`, `/version`, `/metrics` | Never | Liveness, build identity, and per-process metrics must reflect the current process. |
| Everything else (`/`, `/developer`, `/openapi.json`, `/assets/*`) | Managed `CachingOptimized` policy: 1 day by default | Query strings and cookies are not forwarded. Each deploy invalidates `/*`. |

The diagnostic endpoints share the `/???????` path pattern: each has exactly
seven characters after the leading slash, and CloudFront's `?` matches one
character. Other paths of the same length also bypass caching. Combining
these endpoints keeps the distribution at four total cache behaviors, including
the default, within the Free plan's limit of five. Routing and this limit are
checked in `test_diagnostics.py`; keep the pattern aligned when adding diagnostics.

`/v1/*` and `/openapi.json` responses include `Access-Control-Allow-Origin: *` for the GitHub Pages REST reference. The `/v1/*` behavior allows `GET`, `HEAD`, and `OPTIONS`, so browser preflight requests reach the app. Only `GET` and `HEAD` are cached.

CloudFront compresses cacheable responses with gzip or Brotli when the origin does not. On pay-as-you-go, 503 and 504 errors have zero error-cache TTL and the origin read timeout is 60 seconds, above the app's 50-second limit. The Free profile uses AWS defaults: global edge routing, a 30-second origin read timeout and default error caching. A slow origin can therefore return 504 before the app's timeout. Application errors send `no-store`; CloudFront-generated failures can retain the default 10-second error-cache TTL.

### Origin protection

The Function URL stays public (`AuthType NONE`). CloudFront adds `X-Weather-Bridge-Origin-Verify` to every origin request, and CloudFront overwrites any viewer-supplied header with that name. The Lambda receives the same value as `WEATHER_BRIDGE_ORIGIN_VERIFY`. The app rejects requests without the matching header with 403, including direct public `/healthz` requests. The only exemption is `GET`/`HEAD /healthz` with no query string and a loopback Host of `127.0.0.1:8080`, `localhost:8080`, or `[::1]:8080`, for the Lambda Web Adapter readiness check. Public health checks through CloudFront carry the origin header normally. A rejected direct request still invokes Lambda briefly, so it counts toward the guard's request threshold.

The value is a deterrent, not a credential. Anyone who can read the Lambda or CloudFront configuration can see it. To rotate it, delete `target/aws-deployment/origin-verify-secret` and deploy.

CloudFront Origin Access Control was not used. OAC requires `AuthType AWS_IAM`, and Lambda then requires clients to send the SHA-256 of every POST body in `x-amz-content-sha256` ([CloudFront: restrict access to a Lambda function URL origin](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/private-content-restricting-access-to-lambda.html)). Ordinary MCP clients POST to `/mcp` without that header. A CloudFront Function cannot read request bodies. Lambda@Edge could compute the hash, but it has no free allowance and adds a second always-on function.

## Automatic shutdown

Every five minutes, an EventBridge rule invokes `weather-bridge-usage-guard`. For the current UTC calendar month it checks:

- **Lambda**, all functions in the account's enabled regions: stop at **800,000 requests** (80% of the 1,000,000 free requests) or **320,000 estimated GB-seconds** (80% of the 400,000 free GB-seconds).
- **CloudFront**, all distributions in the account (CloudFront's free tier is account-wide): stop at **8,000,000 requests** (80% of 10,000,000) or **800 GB** downloaded (80% of 1 TB).
- Stop if any usage query fails. Monitor errors and failed scheduler delivery also signal a shutdown through CloudWatch/SNS.

A stop sets `weather-bridge-demo` reserved concurrency to 0 first, then disables this stack's CloudFront distribution. A disabled distribution serves no requests. The guard can change only that function's concurrency and only that distribution. It cannot stop other functions or distributions. Normal checks never restart a stopped demo. The usage calculation includes the guard itself and existing resources.

### Compute estimate

Compute is CloudWatch `Duration` × configured memory, plus **10 seconds for each estimated cold start**. Ten seconds is the on-demand Init phase limit. `Duration` excludes Init, and Lambda bills Init for all on-demand functions ([execution environment lifecycle](https://docs.aws.amazon.com/lambda/latest/dg/lambda-runtime-environment.html), [Init billing](https://aws.amazon.com/blogs/compute/aws-lambda-standardizes-billing-for-init-phase/)). Lambda publishes no Init duration or cold-start metric in `AWS/Lambda` ([Lambda metrics](https://docs.aws.amazon.com/lambda/latest/dg/monitoring-metrics-types.html)).

- **The demo:** a CloudWatch Logs metric filter counts the `INIT_START` line (`platform.initStart` in JSON format) that Lambda writes each time it creates an execution environment. The custom metric is `WeatherBridge/DemoInitStarts`. The guard trusts the count only if:
  - the filter exists with the expected pattern and metric;
  - the log group received at least as many events as there were invocations in complete hours ending before `now − 15 minutes`.

  Otherwise, and for invocations before the filter existed, every invocation is charged a cold start. The recent 15–75 minute window and any unmatched invocations are always charged as potential cold starts, so delayed metrics cannot make warm-start savings look larger.
- **Every other function:** every invocation is charged a cold start, as before. This overestimates busy existing functions.

Lambda recycles execution environments every few hours even under continuous load, so a bound based on the minimum environment lifetime is not reliable and is not used.

CloudWatch data can arrive late, and historical memory changes can affect the estimate. Other linked billing accounts, if present, are not queried. This is not an authoritative billing counter or an exact free-tier cap.

### Budget backstop

A **$1/month cost budget for AWS Lambda and Amazon CloudFront** sends an SNS shutdown signal when actual reported charges for those services exceed **1% ($0.01)**. The budget keeps its original name, `weather-bridge-lambda-backstop`. It covers the whole account for those services, not only the demo, but its action stops only the demo. This is a delayed billing backstop, so charges may already have accrued. S3 is not included: artifact storage is a fraction of a cent, and account-wide S3 charges from unrelated buckets would stop the demo permanently. The existing account budget remains unchanged. SNS has no email subscribers; signals invoke the guard directly. Alarm status and logs are visible in the AWS console.

### Concurrency constraint

Reserved concurrency has no charge ([configuring reserved concurrency](https://docs.aws.amazon.com/lambda/latest/dg/configuration-concurrency.html)). Lambda requires at least 100 unreserved concurrency to remain in the account. With the guard's own reservation of 1, the default of 3 needs an account concurrency quota of at least 104, plus any existing reservations. Accounts with a low initial quota (for example 10) cannot reserve any concurrency. If the reservation is rejected, resume fails, the guard reports an error, and the demo stays stopped. Check **Unreserved account concurrency** in the Lambda console before deploying. Higher concurrency raises peak throughput and the aggregate NWS request rate. It does not raise the monthly cost cap.

## Costs

Prices are for us-east-1 as of the ADR date. Recheck before deploying.

| Item | Expected monthly cost | Source |
| --- | --- | --- |
| CloudFront requests and transfer | $0 within 10,000,000 requests and 1 TB (always free). The guard stops at 80%. | [CloudFront pricing](https://aws.amazon.com/cloudfront/pricing/pay-as-you-go/) |
| CloudFront to Lambda origin transfer | $0 (transfer from AWS origins to CloudFront is free) | same |
| Cache invalidation | $0: one wildcard path per deploy; the first 1,000 paths per month are free | same |
| Lambda | $0 within 1,000,000 requests and 400,000 GB-s. Caching reduces invocations. | [Lambda pricing](https://aws.amazon.com/lambda/pricing/) |
| Reserved concurrency | $0 | [configuring reserved concurrency](https://docs.aws.amazon.com/lambda/latest/dg/configuration-concurrency.html) |
| Cold-start metric (one metric-filter metric) | $0 within the 10 free custom metrics; otherwise $0.30 | [CloudWatch pricing](https://aws.amazon.com/cloudwatch/pricing/) |
| Guard metric reads (`GetMetricStatistics`: 2 per Lambda function, 2 per distribution, and up to 5 for the demo's cold-start evidence per run; 8,640 runs) | $0 within 1,000,000 free API requests: about 150,000 with five functions and one distribution. The allowance is shared with other account usage. | same |
| Artifact storage (about 5 MB per build; two builds briefly during a deploy) | < $0.001 at $0.023/GB-month | [S3 pricing](https://aws.amazon.com/s3/pricing/) |
| Cost budget with SNS notification | $0 (budgets without actions are free) | [Budgets pricing](https://aws.amazon.com/aws-cost-management/aws-budgets/pricing/) |
| WAF | $0. None on the default plan. On the Free plan, the plan covers the web ACL. | [flat-rate plans](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/flat-rate-pricing-plan.html) |

**Worst case.** Monitoring, logs, transfer, and allowances shared with other workloads can still produce charges. No zero-dollar bill is guaranteed.

- **Lambda.** Reserved concurrency limits invocation throughput, and throttled requests are not billed. Overshoot during the 5-minute check interval plus metric delay is at most a few hundred thousand requests (about $0.20 per million beyond the free tier). Compute overshoot is under 1,000 GB-s, or $0.0133 per 1,000 GB-s.
- **CloudFront on the default plan.** CloudFront has no rate cap. Usage above the free tier during the guard's reaction window costs $0.01 per 10,000 requests and $0.085/GB. That window is the check interval plus metric delay plus distribution propagation, roughly 20 minutes. Overshoot depends on attacker throughput. A sustained 2,000 requests/s adds about 2.4 million requests in that window, which is still mostly inside the 20% headroom (about $0.40 at most). The guard does not limit cost from a large bandwidth flood. Only the flat-rate plan removes that risk.
- **CloudFront on the Free plan.** $0 for CloudFront and its WAF. AWS does not charge overages; sustained excess may degrade delivery instead.

## CloudFront pricing plan

CloudFront offers flat-rate plans with no overage charges, including a **$0 Free plan**: 1,000,000 requests and 100 GB per month, WAF with 5 rules including IP rate limiting, and DDoS protection. Usage above the allowance is not billed; sustained excess may degrade delivery ([flat-rate plans](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/flat-rate-pricing-plan.html)). Since September 2026, CloudFormation can manage the plan with `AWS::PricingPlanManager::Subscription` ([resource reference](https://docs.aws.amazon.com/AWSCloudFormation/latest/TemplateReference/aws-resource-pricingplanmanager-subscription.html)).

`--cloudfront-plan FREE` (Terraform `cloudfront_pricing_plan = "FREE"`) creates a CLOUDFRONT-scope WAF web ACL with a per-IP rate limit (300 requests per 5 minutes) and a Free plan subscription for the distribution. It requires region us-east-1. Use it if your account is eligible, because it removes the CloudFront overage risk. It is not the default because:

- **Eligibility.** Accounts on the AWS Free account plan are not eligible. Check the account plan in the Billing console first. If the subscription fails, the stack update rolls back and the demo stays stopped.
- **No `/v1` caching.** The Free plan allows only managed cache policies. The managed policies that honor origin `Cache-Control` include `Host` in the cache key. They would forward the CloudFront `Host` to the Function URL, which routes by `Host`. On the Free plan, `/v1/*` is therefore not cached.
- **Five cache behaviors total.** The default behavior counts toward this limit. The diagnostic endpoints share one uncached behavior so this distribution fits.
- **The web ACL is billed if the plan lapses.** It costs $5/month plus $1 per rule ([WAF pricing](https://aws.amazon.com/waf/pricing/)). Keep the subscription and the web ACL together.
- **Leaving the plan takes two steps.** A subscribed distribution cannot drop its web ACL. To return to pay-as-you-go, cancel the Free plan subscription first (it ends immediately), then deploy with `PAY_AS_YOU_GO`.

The rate limit applies only to traffic through CloudFront. Direct Function URL requests skip it. The origin header check rejects them cheaply, but they still count as Lambda invocations.

## Stop, inspect, and resume

Replace `your-profile` in these commands. They act on the demo's guard only:

```sh
aws lambda invoke --function-name weather-bridge-usage-guard --profile your-profile --region us-east-1 --cli-binary-format raw-in-base64-out --payload '{"action":"status"}' /tmp/weather-guard-status.json
aws lambda invoke --function-name weather-bridge-usage-guard --profile your-profile --region us-east-1 --cli-binary-format raw-in-base64-out --payload '{"action":"stop"}' /tmp/weather-guard-stop.json
aws lambda invoke --function-name weather-bridge-usage-guard --profile your-profile --region us-east-1 --cli-binary-format raw-in-base64-out --payload '{"action":"resume"}' /tmp/weather-guard-resume.json
```

Read the output file and check the CLI response for `FunctionError`.

- `status` reports Lambda and CloudFront usage, cold-start evidence (`demoColdStarts`), and `distributionEnabled`.
- `status` can also stop the demo: it sets reserved concurrency to zero and disables the distribution if any usage threshold is reached, or if usage collection or a subsequent configuration read fails. It never resumes a stopped demo. Run it only when that shutdown behavior is intended.
- For read-only evidence, inspect recent `/aws/lambda/weather-bridge-usage-guard` CloudWatch log results, the guard's scheduler and alarms, the demo's reserved concurrency, and the distribution's enabled state. Check the current Lambda and CloudFront metrics and pricing-plan subscription separately; a previous successful log result does not establish current usage or configuration.
- `resume` works only when usage is below every threshold. It restores the configured concurrency and re-enables the distribution, which takes a few minutes to propagate.
- A new month does not resume the demo by itself. Find out why it stopped first.

For an emergency stop without the guard, run `aws lambda put-function-concurrency --function-name weather-bridge-demo --reserved-concurrent-executions 0` with the same profile and region, or disable the distribution in the CloudFront console.

CloudFormation declares the app's concurrency as zero and the distribution as enabled. The guard's changes are expected drift from that. A stack update may reset them, so every deployment ends with a guarded resume.

Before sharing the public endpoint, test these through the CloudFront URL:

- MCP initialization and tool calls;
- wrong-origin rejection;
- direct Function URL rejection;
- cache headers and compressed asset sizes;
- stop and resume.

## Remove

Stop the demo and delete the `weather-bridge-demo` stack. CloudFormation disables and then deletes the distribution, which can take several minutes. On the Free plan, it removes the subscription first. Remove artifact objects before deleting `weather-bridge-artifacts`; its bucket cannot be deleted while nonempty. Check the stack events for cleanup failures. Removing these stacks removes their functions, distribution, cache policy, metric filter, rules, alarms, SNS topic, budget, roles, and logs. It does not remove existing account resources.

## Custom hostname with external DNS

Give the demo its own subdomain, for example `bridge.weather.example.com`, and leave other records on the parent domain alone. Public ACM certificates used with CloudFront are free and live in `us-east-1`. Terraform owns the certificate, CloudFront alias and viewer certificate. DNS stays with your current provider. Keep the validation CNAME, or the certificate can't renew.

Request the certificate before changing the running demo:

```sh
python3 infra/aws/deploy.py --profile your-profile --region us-east-1 --contact admin@example.com --release "target/releases/$REVISION" --revision "$REVISION" --cloudfront-plan FREE --domain bridge.weather.example.com --prepare-domain
terraform -chdir=infra/aws/terraform output -json certificate_dns_validation_records > target/aws-deployment/certificate-dns.json
```

This step creates only the certificate. It does not pause the app or change CloudFront. Add the printed validation CNAME at your DNS provider and wait for ACM status `ISSUED`.

For DreamHost, an optional helper adds the records for you:

- It prints a plan unless you pass `--apply`.
- It refuses to overwrite conflicting records and never removes records.
- It only adds the selected hostname or its ACM validation CNAME.

Set `DREAMHOST_API_KEY` from your credential manager, limited to `dns-list_records` and `dns-add_record`, then run:

```sh
python3 infra/aws/dns-dreamhost.py --domain bridge.weather.example.com --validation-file target/aws-deployment/certificate-dns.json --apply
```

Once the certificate is issued, rerun the deployment without `--prepare-domain`. The driver confirms the certificate before pausing the demo, sets the MCP origin to the custom hostname, and configures CloudFront HTTPS with SNI and TLS 1.2 or newer:

```sh
python3 infra/aws/deploy.py --profile your-profile --region us-east-1 --contact admin@example.com --release "target/releases/$REVISION" --revision "$REVISION" --cloudfront-plan FREE --domain bridge.weather.example.com
terraform -chdir=infra/aws/terraform output -raw distribution_endpoint
python3 infra/aws/dns-dreamhost.py --domain bridge.weather.example.com --target dexample.cloudfront.net --apply
```

Then:

1. Replace `dexample.cloudfront.net` with your distribution hostname.
2. Create the final CNAME only after the distribution update finishes. With another DNS provider, add both CNAMEs by hand.
3. Update the docs' API URL only after HTTPS and MCP work on the new hostname.

Later driver runs keep the deployed hostname and pricing plan if you leave out `--domain` or `--cloudfront-plan`. The driver handles a first hostname and redeploys of the same hostname. Moving to a different hostname needs a separate, staged certificate migration; the driver rejects that change before touching the demo.
