resource "newrelic_alert_policy" "releash" {
  account_id          = var.newrelic_account_id
  name                = "Releash observability"
  incident_preference = "PER_CONDITION"
}

resource "newrelic_nrql_alert_condition" "repo_snapshot_budget" {
  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Repo snapshot P95 over 300ms"
  description                  = "Detects M0 repo snapshot budget violations for git.status_scan."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.aggregation_window_seconds
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE ${local.operation_filter.repo_snapshot}"
  }

  critical {
    operator              = "above"
    threshold             = local.budget.repo_snapshot_p95_ms
    threshold_duration    = local.alert_counts.sustained_window_seconds
    threshold_occurrences = "ALL"
  }
}

resource "newrelic_nrql_alert_condition" "crash_spike" {
  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Crash or exception spike"
  description                  = "Detects at least five Releash crash or exception logs in five minutes."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.crash_window_min * 60
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Log SELECT count(*) WHERE ${local.crash_error_filter}"
  }

  critical {
    operator              = "above_or_equals"
    threshold             = local.alert_counts.crash_threshold
    threshold_duration    = local.alert_counts.crash_window_min * 60
    threshold_occurrences = "AT_LEAST_ONCE"
  }
}
