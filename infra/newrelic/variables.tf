variable "newrelic_account_id" {
  description = "New Relic account ID that owns the Releash observability resources."
  type        = number
}

variable "newrelic_api_key" {
  description = "New Relic User API key. Supply via TF_VAR_newrelic_api_key or CI secrets."
  type        = string
  sensitive   = true
}

variable "newrelic_region" {
  description = "New Relic account region."
  type        = string
  default     = "US"

  validation {
    condition     = contains(["US", "EU", "JP"], upper(var.newrelic_region))
    error_message = "newrelic_region must be one of US, EU, or JP."
  }
}

variable "dashboard_permissions" {
  description = "Visibility for the Releash New Relic dashboard."
  type        = string
  default     = "private"

  validation {
    condition = contains([
      "private",
      "public_read_only",
      "public_read_write",
    ], var.dashboard_permissions)
    error_message = "dashboard_permissions must be private, public_read_only, or public_read_write."
  }
}
