# Terraform: Weather Bridge on AWS

Terraform manages the two CloudFormation stacks, their versioned build artifacts, and the optional ACM certificate for a custom domain. CloudFormation owns the individual resources, all defined in the adjacent checked-in JSON templates: CloudFront distribution and cache policy, Lambda, IAM, logs and metric filter, rule, alarm, SNS, budget, and, on the optional Free plan, the WAF web ACL and plan subscription. Each resource has one owner, so do not also import the stack resources into Terraform.

Use the [deployment driver](../deploy.py) and [operations guide](../README.md). The driver:

1. Verifies and stages `app.zip`, `guard.zip`, and their release manifest before any AWS or Terraform command.
2. Generates the CloudFront origin-verification value once and keeps it in ignored `target/aws-deployment/`.
3. Writes ignored local Terraform settings and initializes the locked AWS provider.
4. Imports existing Weather Bridge stacks when adopting an existing deployment.
5. Saves a Terraform plan, verifies its resource addresses, and applies that exact plan.
6. Configures the generated CloudFront origin and Function URL host on a second apply when they change.
7. Invalidates the CloudFront cache, then calls the guard to resume only after current account usage passes its checks.

Build and package the release with `scripts/release.py` before planning; the binary embeds the city data. Scalar is used only by the separate Pages build. Build artifacts stay under ignored `target/`. Every plan shows the origin-verification parameter as changed; see [Known plan difference](../README.md#known-plan-difference). The guard checks usage before enabling the demo.

Artifact objects are content-addressed and replaced create-before-destroy. Terraform deletes the previous object only after the stack update that stops using it, so CloudFormation always has the current code object for rollback. The bucket has no expiry rule.

## Inputs

| Variable | Purpose |
| --- | --- |
| `nws_contact` | Public contact sent to NWS in the User-Agent. |
| `public_origin` | CloudFront HTTPS origin (from `endpoint`). Empty only during disabled bootstrap. |
| `origin_host` | Function URL host (from `function_url`); the MCP Host allowlist includes it because CloudFront sends the origin's Host to Lambda. |
| `origin_verify_secret` | Sensitive. Value CloudFront sends as `X-Weather-Bridge-Origin-Verify`. |
| `reserved_concurrency` | 1–10, default 3. Concurrency the guard restores on resume. |
| `custom_domain` | Optional public hostname. Terraform requests its ACM certificate in us-east-1; retain the DNS validation records for renewal. See the [staged custom-domain setup](../README.md#custom-hostname-with-external-dns). |
| `cloudfront_pricing_plan` | `PAY_AS_YOU_GO` (default) or `FREE`. `FREE` requires region us-east-1. |

## Manual inspection

After a driver deployment:

```sh
terraform -chdir=infra/aws/terraform fmt -check
terraform -chdir=infra/aws/terraform validate
terraform -chdir=infra/aws/terraform plan
terraform -chdir=infra/aws/terraform output endpoint
```

For a fresh setup, `terraform.tfvars.example` documents required inputs. Copy it to `deployment.auto.tfvars` or set `TF_VAR_*` variables. Do not use direct apply to bypass the guarded bootstrap/resume procedure.

## Public repository boundaries

Commit `.tf` files, the provider lock file `.terraform.lock.hcl`, placeholder examples, and the JSON templates. Do not commit Terraform state, `.terraform/`, actual `.tfvars`, saved plans, credentials, the origin-verification value, or local deployment metadata. The root `.gitignore` excludes these. Never store credentials in Terraform variables or templates; use an authenticated AWS CLI profile or the standard AWS credential chain.

State defaults to local storage. Back it up privately. Shared/team deployment should configure an encrypted remote backend with locking; do not use the artifact bucket for state. Contact information, the origin-verification value, and deployment/account identifiers can appear in state and plans even though examples remain generic.

Destroying the module removes the demo and its controls. Stop the app first. The artifact bucket must be empty before its stack can be removed; do not bypass this by deleting unrelated account resources. Check Terraform and CloudFormation cleanup results.
