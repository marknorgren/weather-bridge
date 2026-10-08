terraform {
  required_version = ">= 1.6, < 2.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 6.0"
    }
  }
}

provider "aws" {
  region  = var.region
  profile = var.profile
}

locals {
  repository = abspath("${path.module}/../../..")
  app_zip    = "${local.repository}/target/aws-deployment/app.zip"
  guard_zip  = "${local.repository}/target/aws-deployment/guard.zip"
  origin     = var.public_origin == "" ? "http://127.0.0.1:8080" : var.public_origin
  # MCP Host allowlist: the CloudFront domain, the Function URL host that Lambda sees
  # (CloudFront does not forward the viewer Host), and loopback for local checks.
  host = join(",", compact([
    var.public_origin == "" ? "" : trimsuffix(trimprefix(var.public_origin, "https://"), ":443"),
    var.origin_host,
    "127.0.0.1:8080",
    "localhost:8080",
  ]))
}

# CloudFront custom-domain certificates must be in us-east-1. Request and validate
# this certificate before updating the distribution; deploy.py performs that staging.
# DNS remains operator-managed and is never changed by this module.
resource "aws_acm_certificate" "public" {
  count             = var.custom_domain == "" ? 0 : 1
  domain_name       = var.custom_domain
  validation_method = "DNS"
  tags              = { Project = "weather-bridge" }

  lifecycle {
    create_before_destroy = true
    precondition {
      condition     = var.region == "us-east-1"
      error_message = "CloudFront custom-domain certificates require region us-east-1."
    }
  }
}

# Terraform owns the two stacks. CloudFormation owns their individual resources.
# This preserves the running deployment without duplicate resource ownership.
resource "aws_cloudformation_stack" "artifacts" {
  name          = "weather-bridge-artifacts"
  template_body = file("${path.module}/../artifacts.json")
  tags          = { Project = "weather-bridge" }
}

resource "aws_s3_object" "app" {
  bucket                 = aws_cloudformation_stack.artifacts.outputs["Bucket"]
  key                    = "${filesha256(local.app_zip)}.zip"
  source                 = local.app_zip
  source_hash            = filesha256(local.app_zip)
  server_side_encryption = "AES256"
  content_type           = "application/zip"

  # Keep the previous object until the stack update that stops using it has
  # finished. CloudFormation needs that object to roll back a failed update.
  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_s3_object" "guard" {
  bucket                 = aws_cloudformation_stack.artifacts.outputs["Bucket"]
  key                    = "${filesha256(local.guard_zip)}.zip"
  source                 = local.guard_zip
  source_hash            = filesha256(local.guard_zip)
  server_side_encryption = "AES256"
  content_type           = "application/zip"

  # Keep the previous object until the stack update that stops using it has
  # finished. CloudFormation needs that object to roll back a failed update.
  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_cloudformation_stack" "demo" {
  name          = "weather-bridge-demo"
  template_body = file("${path.module}/../template.json")
  capabilities  = ["CAPABILITY_IAM"]
  parameters = {
    ArtifactBucket        = aws_cloudformation_stack.artifacts.outputs["Bucket"]
    AppKey                = aws_s3_object.app.key
    GuardKey              = aws_s3_object.guard.key
    NwsUserAgent          = "WeatherBridge/0.1 (+${var.nws_contact})"
    PublicHost            = local.host
    PublicOrigin          = local.origin
    ReservedConcurrency   = var.reserved_concurrency
    OriginVerifySecret    = var.origin_verify_secret
    CloudFrontPricingPlan = var.cloudfront_pricing_plan
    CustomDomain          = var.custom_domain
    CertificateArn        = var.custom_domain == "" ? "" : aws_acm_certificate.public[0].arn
  }
  tags = { Project = "weather-bridge" }

  lifecycle {
    precondition {
      # CLOUDFRONT-scope WAF web ACLs, which the Free plan requires, exist only in us-east-1.
      condition     = var.cloudfront_pricing_plan != "FREE" || var.region == "us-east-1"
      error_message = "The CloudFront Free plan requires region us-east-1."
    }
  }
}

output "endpoint" {
  value       = aws_cloudformation_stack.demo.outputs["Endpoint"]
  description = "Preferred public HTTPS endpoint: custom_domain when configured, otherwise the CloudFront domain. Use its origin as public_origin."
}

output "function_url" {
  value       = aws_cloudformation_stack.demo.outputs["FunctionUrl"]
  description = "Lambda Function URL behind CloudFront. Use its host as origin_host on the second apply."
}

output "distribution_id" {
  value = aws_cloudformation_stack.demo.outputs["DistributionId"]
}

output "guard_function" {
  value = aws_cloudformation_stack.demo.outputs["GuardFunction"]
}

output "distribution_endpoint" {
  value       = aws_cloudformation_stack.demo.outputs["DistributionEndpoint"]
  description = "Generated CloudFront HTTPS endpoint, including when a custom domain is configured. Use its host as the custom domain's CNAME target."
}

output "certificate_arn" {
  value       = var.custom_domain == "" ? null : aws_acm_certificate.public[0].arn
  description = "Optional custom-domain certificate ARN. Complete DNS validation and wait for ISSUED before updating CloudFront."
}

output "certificate_dns_validation_records" {
  value = var.custom_domain == "" ? [] : [
    for record in aws_acm_certificate.public[0].domain_validation_options : {
      domain = record.domain_name
      name   = record.resource_record_name
      type   = record.resource_record_type
      value  = record.resource_record_value
    }
  ]
  description = "Operator-managed ACM DNS validation records. Retain these records for certificate renewal; Terraform does not manage DNS."
}
