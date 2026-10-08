# ADR-0001: Host the public Weather Bridge demo

## Status

Accepted, 2026-09-30. Provider limits were checked on this date.

- Scope: a public hobby demo.
- AWS deployment with usage controls is approved. Terraform is the deployment entry point.
- The deployment manages only Weather Bridge resources. It leaves other account resources and budgets alone.

Amended by [ADR-0002](adr-0002-cloudfront-origin-and-cost-guard.md): a CloudFront distribution becomes the single public origin in front of the Function URL. The usage guard also caps CloudFront usage, and reserved concurrency becomes configurable.

## Context

At the time of this decision, Weather Bridge was one native Rust/Axum process serving the weather UI, embedded Scalar documentation, REST endpoints, and a stateless MCP HTTP endpoint. GeoNames city data and browser assets are built into the binary. NWS requests, caching, and throttling run in process. There is no database or required persistent disk.

The preferred providers are Cloudflare or AWS. We want a public HTTPS demo with an ongoing free allowance that keeps the shared Rust weather service and makes costs visible. Free compute does not make the whole deployment free.

### Requirements

- Serve `/`, `/developer`, `/docs/rest`, `/openapi.json`, `/v1/*`, and `/mcp`.
- Allow outbound HTTPS to NWS and retain attribution and source-failure reporting.
- Keep the API and MCP behavior consistent; test real MCP clients through the hosting adapter.
- Preserve bounded requests, deadlines, and upstream fair use. Control aggregate traffic across instances.
- Prefer one origin, no database, and infrastructure that can be reproduced from the repository.
- Accept cold starts for a demo, but measure their effect on MCP initialization and calls.

### Options evaluated

| Option | Free allowance or minimum charge | Fit and limitation |
| --- | --- | --- |
| **AWS Lambda + Function URL** | Lambda publishes **1 million requests and 400,000 GB-seconds/month** free. Function URLs have no separate endpoint charge. | Native Rust via Lambda Web Adapter; preserves Axum. Cold starts, account eligibility, logs, transfer, and packaging costs still matter. [Lambda pricing](https://aws.amazon.com/lambda/pricing/), [Function URLs](https://docs.aws.amazon.com/lambda/latest/dg/furls-http-invoke-decision.html). |
| **Cloudflare Workers Free** | **100,000 requests/day, 10 ms CPU/request, 128 MB memory**. | Rust runs through WebAssembly/workers-rs, rather than the existing native server. Requires adapting networking, runtime, cache, and MCP transport; benchmark city-index initialization and CPU limits. [Limits](https://developers.cloudflare.com/workers/platform/limits/), [Rust support](https://developers.cloudflare.com/workers/languages/rust/). |
| **Cloudflare Containers** | Requires **Workers Paid, minimum $5/month**, with included container usage and metered overages. No Containers allowance on Workers Free. | Closest Cloudflare fit for the existing server. Container packaging and Worker routing needed; not a zero-dollar option. [Container pricing](https://developers.cloudflare.com/containers/platform/pricing/), [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/). |
| **AWS EC2 / ECS Fargate** | New-account promotional credits can cover eligible usage temporarily; not an ongoing free container-hosting allocation. | Runs the native server, but adds infrastructure and networking costs. EC2's older 12-month offer applies only to accounts created before July 15, 2025. [EC2 eligibility](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-free-tier-usage.html), [AWS Free Tier](https://aws.amazon.com/free/free-tier-faqs/), [Fargate comparison](https://aws.amazon.com/compare/app-runner-and-fargate/). |
| **Render Free** | **750 instance-hours/workspace/month**; sleeps after **15 minutes** idle, with roughly one-minute wake-up. | Native hosting with no code changes. Cold starts can exceed client timeouts. Bandwidth/build allowances and external-traffic limits apply. [Free services](https://render.com/docs/free). |
| **Koyeb Free Instance** | One instance/organization: **512 MB RAM, 0.1 vCPU, 2 GB SSD**; sleeps after **one hour** idle. | Native hosting alternative in Washington, D.C. or Frankfurt. Card required; current signup documentation describes a verification hold and a default paid Pro plan. Confirm Starter/Free selection and any initial charge before signup. [Instances](https://www.koyeb.com/docs/reference/instances), [Billing FAQ](https://www.koyeb.com/docs/faqs/pricing). |
| **Railway Free** | **$5 trial credit/up to 30 days**, then **$1 credit/month**; free service limit 0.5 GB RAM/1 vCPU. | Native hosting, but a credit budget does not guarantee full-month operation. Limited trial accounts have outbound-network restrictions relevant to NWS. [Pricing](https://railway.com/pricing), [Trial restrictions](https://docs.railway.com/pricing/free-trial). |
| **Google Cloud Run** | Request-based allowance: **2 million requests, 180,000 vCPU-seconds, 360,000 GiB-seconds/month**, valued at us-central1 pricing. | Native container alternative. Billing account required; image storage, builds, and networking have separate allowances/costs. [Pricing](https://cloud.google.com/run/pricing), [Free Tier](https://docs.cloud.google.com/free/docs/free-cloud-features). |
| **Oracle Always Free VM** | A1 allocation: **1,500 OCPU-hours and 9,000 GB-hours/month**, equivalent to 2 OCPUs/12 GB for Always Free tenancies. | Native server, but requires VM/TLS maintenance; capacity constraints and idle-instance reclamation make it a weak fit for an occasional demo. [Always Free resources](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm). |
| **Fly.io** | Trial: **2 VM-hours or seven days**, whichever ends first. | Native deployment, but no ongoing free allowance for this demo. [Trial terms](https://fly.io/docs/about/free-trial/). |
| **GitHub Pages / static hosting** | GitHub Pages is available for public repositories on GitHub Free. | Hosts exported browser assets/docs; needs a separate backend for weather and MCP. Splitting origins adds API URL and CORS configuration. [GitHub Pages](https://docs.github.com/en/pages/getting-started-with-github-pages/what-is-github-pages). |

**AWS account terms:** New customers receive $100 signup credit and may earn another $100. The Free account plan ends after six months or credit depletion; this differs from Lambda's recurring service allowance. Confirm the actual account's plan and eligibility before deployment. On a paid account, usage outside applicable free allowances is billable. [AWS account plans](https://aws.amazon.com/about-aws/whats-new/2025/07/aws-free-tier-credits-month-free-plan/).

App Runner is excluded from new deployment choices: AWS has closed it to new customers and directs customers toward ECS Express Mode, whose underlying resources remain billable. [Availability change](https://docs.aws.amazon.com/apprunner/latest/dg/apprunner-availability-change.html).

## Decision

Adopt **AWS Lambda with a Function URL and Lambda Web Adapter** for the first public demo, keeping all routes on one origin; retain **Cloudflare Containers** as the alternative if a $5/month platform minimum is acceptable.

Use Cloudflare Workers Free only after a separate port demonstrates that the complete weather and MCP implementation fits its runtime and resource limits.

## Rationale

Lambda combines the preferred provider, an ongoing compute allowance, and reuse of the native Rust server. AWS's Web Adapter supports existing HTTP applications and provides an Axum example. The deployed REST and MCP endpoints passed the live smoke check (`scripts/smoke.py --url … --live`). [Lambda Web Adapter](https://github.com/aws/aws-lambda-web-adapter).

Cloudflare Containers preserves the same implementation with simpler runtime assumptions, but introduces a monthly minimum. Workers Free fits a later edge-native version. Changing runtimes only to save the platform fee would grow the demo's scope. Render and Koyeb remain fallbacks.

## Implementation

The implementation is documented in [infra/aws/README.md](../infra/aws/README.md), with [Terraform configuration](../infra/aws/terraform/README.md) and checked-in resource templates. Usage controls stop only the demo; they do not guarantee a zero-dollar bill. ADR-0002 adds CloudFront caching, origin verification, and CloudFront usage limits. These address step 4's rate-limit and aggregate-traffic concerns within the free allowances.

1. Package the locked Rust release and a pinned Lambda Web Adapter using infrastructure as code. Start without a database, provisioned concurrency, VPC/NAT gateway, or API Gateway. Do not add those services without a concrete requirement and cost review.
2. Bind the application to the adapter's port; use `/healthz` for readiness. Keep the existing deadlines and set the Lambda timeout above the application's 50-second HTTP limit.
3. Configure exact public MCP hosts and HTTPS origins in the application; preserve Host/Origin validation. Set an operator-selected `WEATHER_BRIDGE_USER_AGENT` with contact information.
4. Start with low concurrency, add incoming rate limits, and check aggregate NWS request rate. Per-process cache/limiter state is not shared and may disappear between invocations; raising concurrency requires a fleet-wide upstream budget.
5. Test the buffered JSON MCP flow, initialization notifications, tool discovery, structured errors, and weather calls through the adapter. Scalar is published separately on GitHub Pages. Exercise cold starts and verify payload sizes. Validate SSE separately before promising persistent streaming or server notifications.
6. Configure short log retention, billing alerts, and usage monitoring. Alerts are not a hard spending cap. Check packaging/storage, transfer, and logging charges separately from Lambda compute.

## Consequences

- One API origin serves the weather UI and MCP explorer. The separate Pages REST reference calls it through public read-only CORS.
- The native implementation remains portable to container hosts.
- Cold starts and NWS latency may affect client deadlines; no always-on or availability promise is made.
- A popular public endpoint can exceed free allowances. Metering and rate limits are part of deployment work.
- REST/MCP suitability and shutdown delivery passed public deployment tests. Actual long-term costs remain to be measured.

## Success metrics

Before sharing the public URL, verify all routes over HTTPS, both browser explorers, and at least one external MCP client. Measure cold/warm latency and memory; test wrong-origin rejection and rate-limit behavior. Record seven days of invocations, compute, transfer, and logs against the actual account's allowances before describing operation as free.

## References

Provider sources are linked beside each claim. Recheck pricing at implementation time. Application constraints are documented in [README.md](../README.md), with transport configuration in [src/api/](../src/api/mod.rs).
