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

resource "newrelic_nrql_alert_condition" "session_io_budget" {
  for_each = toset(local.session_io_operations)

  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Session IO P95 over 300ms (${each.value})"
  description                  = "Detects M0 AgentSession IO budget violations for ${each.value}."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.aggregation_window_seconds
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE `${local.attr.operation}` = '${each.value}'"
  }

  critical {
    operator              = "above"
    threshold             = local.budget.repo_snapshot_p95_ms
    threshold_duration    = local.alert_counts.sustained_window_seconds
    threshold_occurrences = "ALL"
  }
}

resource "newrelic_nrql_alert_condition" "diff_open_budget" {
  for_each = toset(local.diff_open_operations)

  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Diff open P95 over 500ms (${each.value})"
  description                  = "Detects M0 diff open budget violations for ${each.value}."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.aggregation_window_seconds
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE `${local.attr.operation}` = '${each.value}'"
  }

  critical {
    operator              = "above"
    threshold             = local.budget.diff_open_p95_ms
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

resource "newrelic_nrql_alert_condition" "ws_reconnects" {
  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "WebSocket reconnect spike"
  description                  = "Detects at least five agent stream WebSocket reconnects in five minutes."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.quality_window_min * 60
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT sum(`${local.metrics.ws_reconnects}`)"
  }

  critical {
    operator              = "above_or_equals"
    threshold             = local.alert_counts.ws_reconnects_threshold
    threshold_duration    = local.alert_counts.quality_window_min * 60
    threshold_occurrences = "AT_LEAST_ONCE"
  }
}

resource "newrelic_nrql_alert_condition" "dropped_frames" {
  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Agent stream dropped frame spike"
  description                  = "Detects at least ten dropped agent stream frames in five minutes."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.quality_window_min * 60
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT sum(`${local.metrics.dropped_frames}`)"
  }

  critical {
    operator              = "above_or_equals"
    threshold             = local.alert_counts.dropped_frames_threshold
    threshold_duration    = local.alert_counts.quality_window_min * 60
    threshold_occurrences = "AT_LEAST_ONCE"
  }
}

resource "newrelic_nrql_alert_condition" "payload_budget" {
  account_id                   = var.newrelic_account_id
  policy_id                    = newrelic_alert_policy.releash.id
  type                         = "static"
  name                         = "Agent stream payload P95 over 64KB"
  description                  = "Detects sustained agent stream payload frames over the 64KB budget."
  enabled                      = true
  violation_time_limit_seconds = local.violation_time_limit_seconds
  aggregation_window           = local.alert_counts.aggregation_window_seconds
  aggregation_method           = "event_flow"
  aggregation_delay            = local.alert_counts.aggregation_delay_seconds

  nrql {
    query = "FROM Metric SELECT percentile(`${local.metrics.payload_bytes}`, 95) WHERE `${local.attr.channel}` IN (${join(", ", [for channel in local.stream_channels : "'${channel}'"])})"
  }

  critical {
    operator              = "above"
    threshold             = local.budget.payload_bytes
    threshold_duration    = local.alert_counts.sustained_window_seconds
    threshold_occurrences = "ALL"
  }
}
