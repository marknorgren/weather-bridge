variable "profile" {
  type        = string
  default     = null
  description = "AWS CLI profile, or null to use the standard AWS credential chain."
}

variable "region" {
  type    = string
  default = "us-east-1"
}

variable "nws_contact" {
  type        = string
  description = "Public project URL or email sent to NWS in the User-Agent."
  validation {
    condition     = length(trimspace(var.nws_contact)) > 0
    error_message = "A public NWS contact is required."
  }
}

variable "public_origin" {
  type        = string
  default     = ""
  description = "Exact CloudFront HTTPS origin without trailing slash; empty only during disabled bootstrap."
  validation {
    condition     = var.public_origin == "" || can(regex("^https://[a-zA-Z0-9.-]+(:443)?$", var.public_origin))
    error_message = "Use an exact HTTPS origin, such as https://example.com:443."
  }
}

variable "reserved_concurrency" {
  type        = number
  default     = 3
  description = "Reserved concurrency the usage guard restores on resume. Requires 100 + this value of unreserved account concurrency."
  validation {
    condition     = var.reserved_concurrency >= 1 && var.reserved_concurrency <= 10 && floor(var.reserved_concurrency) == var.reserved_concurrency
    error_message = "Use a whole number from 1 to 10."
  }
}

variable "origin_host" {
  type        = string
  default     = ""
  description = "Lambda Function URL host that the app sees behind CloudFront; empty only during disabled bootstrap."
  validation {
    condition     = var.origin_host == "" || can(regex("^[a-z0-9]+\\.lambda-url\\.[a-z0-9-]+\\.on\\.aws$", var.origin_host))
    error_message = "Use the Function URL host, such as abc123.lambda-url.us-east-1.on.aws."
  }
}

variable "origin_verify_secret" {
  type        = string
  sensitive   = true
  description = "Value CloudFront sends in X-Weather-Bridge-Origin-Verify; the app rejects direct requests without it. deploy.py generates and keeps it in ignored local state."
  validation {
    condition     = can(regex("^[A-Za-z0-9_-]{32,128}$", var.origin_verify_secret))
    error_message = "Use 32-128 URL-safe characters."
  }
}

variable "cloudfront_pricing_plan" {
  type        = string
  default     = "PAY_AS_YOU_GO"
  description = "PAY_AS_YOU_GO (always-free tier, usage guard) or FREE ($0 flat-rate plan with WAF rate limit; see infra/aws/README.md)."
  validation {
    condition     = contains(["PAY_AS_YOU_GO", "FREE"], var.cloudfront_pricing_plan)
    error_message = "Use PAY_AS_YOU_GO or FREE."
  }
}

variable "custom_domain" {
  type        = string
  default     = ""
  description = "Optional lowercase public DNS hostname for CloudFront. Requires region us-east-1 and operator-managed ACM DNS validation before distribution apply. Empty keeps the generated CloudFront hostname."
  validation {
    condition = var.custom_domain == "" || (
      length(var.custom_domain) <= 253 &&
      can(regex("^([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\\.)+[a-z]([a-z0-9-]{0,61}[a-z0-9])?$", var.custom_domain))
    )
    error_message = "Use a lowercase fully qualified hostname, such as api.example.com, without a scheme, port, wildcard, or trailing dot."
  }
}
