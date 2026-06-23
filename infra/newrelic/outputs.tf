output "dashboard_guid" {
  description = "GUID of the Releash New Relic dashboard."
  value       = newrelic_one_dashboard.releash.guid
}

output "dashboard_permalink" {
  description = "Permalink for the Releash New Relic dashboard."
  value       = newrelic_one_dashboard.releash.permalink
}

output "alert_policy_id" {
  description = "ID of the Releash New Relic alert policy."
  value       = newrelic_alert_policy.releash.id
}
