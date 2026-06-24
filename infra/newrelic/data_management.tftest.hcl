mock_provider "newrelic" {}

variables {
  newrelic_account_id = 123456
  newrelic_api_key    = "test-key"
}

run "metric_pruning_denylist_preserves_allowed_resource_attributes" {
  command = plan

  assert {
    condition = length(local.allowed_resource_attrs) == 4 && toset(local.allowed_resource_attrs) == toset([
      "service.version",
      "os.type",
      "releash.build_type",
      "service.name",
    ])

    error_message = "Allowed resource attributes must stay fixed to the #1209 allowlist."
  }

  assert {
    condition = length(setintersection(toset(local.allowed_resource_attrs), toset(local.pii_attribute_keys))) == 0

    error_message = "Metric pruning deny-list must not drop allowed resource attributes."
  }
}

run "metric_pruning_rule_is_scoped_to_releash" {
  command = plan

  assert {
    condition = newrelic_metric_pruning_rule.pii_metric_aggregates.nrql == "SELECT ${local.pii_delete_clause} FROM Metric WHERE ${local.metric_releash_scope}"

    error_message = "Metric pruning must be scoped to Releash metrics."
  }
}

run "metric_pruning_targets_exported_high_volume_signal" {
  command = plan

  assert {
    condition = newrelic_metric_pruning_rule.pii_metric_aggregates.nrql == "SELECT ${local.pii_delete_clause} FROM Metric WHERE ${local.metric_releash_scope}"

    error_message = "Free-tier ingest control must target the Metric signal exported by #1209."
  }

  assert {
    condition = strcontains(newrelic_metric_pruning_rule.pii_metric_aggregates.description, "#1209 Releash dimensional metric aggregates") && strcontains(newrelic_metric_pruning_rule.pii_metric_aggregates.description, "free-tier ingest control")

    error_message = "Metric pruning must document that exported metric aggregates, not unexported debug logs, carry the free-tier ingest control responsibility."
  }
}

run "duration_budget_alerts_are_scoped_per_operation" {
  command = plan

  assert {
    condition = toset(keys(newrelic_nrql_alert_condition.session_io_budget)) == toset(local.session_io_operations)

    error_message = "Session IO budget alerts must be generated for every session operation."
  }

  assert {
    condition = alltrue([
      for op in local.session_io_operations :
      newrelic_nrql_alert_condition.session_io_budget[op].nrql[0].query == "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE `${local.attr.operation}` = '${op}'"
    ])

    error_message = "Session IO budget alerts must evaluate exactly one operation per condition."
  }

  assert {
    condition = alltrue([
      for op in local.session_io_operations :
      newrelic_nrql_alert_condition.session_io_budget[op].critical[0].threshold == local.budget.repo_snapshot_p95_ms
    ])

    error_message = "Session IO budget alerts must keep the 300ms repo snapshot/session IO budget."
  }

  assert {
    condition = toset(keys(newrelic_nrql_alert_condition.diff_open_budget)) == toset(local.diff_open_operations)

    error_message = "Diff open budget alerts must be generated for every diff open operation."
  }

  assert {
    condition = alltrue([
      for op in local.diff_open_operations :
      newrelic_nrql_alert_condition.diff_open_budget[op].nrql[0].query == "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE `${local.attr.operation}` = '${op}'"
    ])

    error_message = "Diff open budget alerts must evaluate exactly one operation per condition."
  }

  assert {
    condition = alltrue([
      for op in local.diff_open_operations :
      newrelic_nrql_alert_condition.diff_open_budget[op].critical[0].threshold == local.budget.diff_open_p95_ms
    ])

    error_message = "Diff open budget alerts must keep the 500ms diff open budget."
  }
}

run "agent_turn_latency_definitions_are_canonical" {
  command = plan

  assert {
    condition = local.metrics.agent_turn == "releash.agent.turn.duration_ms"

    error_message = "Agent turn latency metric name must match the Rust telemetry histogram."
  }

  assert {
    condition = local.agent_turn_operations == [
      "agent.turn.ui_to_start",
      "agent.turn.bridge_spawn",
      "agent.turn.query_init",
      "agent.turn.first_sdk_event",
      "agent.turn.first_assistant_event",
      "agent.turn.permission_wait",
      "agent.turn.complete",
    ]

    error_message = "Agent turn operations must match the Rust AgentTurnMetric operations."
  }

  assert {
    condition = local.operation_filter.agent_turn == "`${local.attr.operation}` IN (${join(", ", [for op in local.agent_turn_operations : "'${op}'"])})"

    error_message = "Agent turn operation filter must include every turn operation."
  }

  assert {
    condition = toset([
      local.attr.agent_resume,
      local.attr.agent_has_session,
      local.attr.agent_permission_mode,
      local.attr.agent_model,
      local.attr.agent_context,
      local.attr.agent_warm_path,
      ]) == toset([
      "releash.agent.resume",
      "releash.agent.has_session",
      "releash.agent.permission_mode",
      "releash.agent.model",
      "releash.agent.context",
      "releash.agent.warm_path",
    ])

    error_message = "Agent turn dimension attributes must match the Rust releash.agent.* keys."
  }

  assert {
    condition = length(setintersection(toset([
      local.attr.agent_resume,
      local.attr.agent_has_session,
      local.attr.agent_permission_mode,
      local.attr.agent_model,
      local.attr.agent_context,
      local.attr.agent_warm_path,
    ]), toset(local.pii_attribute_keys))) == 0

    error_message = "Agent turn dimensions must not be removed by PII metric pruning."
  }

  assert {
    condition = length([
      for widget in newrelic_one_dashboard.releash.page[0].widget_line :
      widget.nrql_query[0].query
      if widget.title == "Agent turn latency P50 by operation x context"
      ]) == 1 && alltrue([
      for query in [
        for widget in newrelic_one_dashboard.releash.page[0].widget_line :
        widget.nrql_query[0].query
        if widget.title == "Agent turn latency P50 by operation x context"
      ] :
      strcontains(query, "percentile(`${local.metrics.agent_turn}`, 50)") &&
      strcontains(query, "WHERE ${local.operation_filter.agent_turn}") &&
      strcontains(query, "FACET `${local.attr.operation}`, `${local.attr.agent_context}`")
    ])

    error_message = "Agent turn P50 dashboard widget must keep percentile, WHERE operation filter, and operation x context FACET."
  }

  assert {
    condition = length([
      for widget in newrelic_one_dashboard.releash.page[0].widget_line :
      widget.nrql_query[0].query
      if widget.title == "Agent turn latency P95 by operation x context"
      ]) == 1 && alltrue([
      for query in [
        for widget in newrelic_one_dashboard.releash.page[0].widget_line :
        widget.nrql_query[0].query
        if widget.title == "Agent turn latency P95 by operation x context"
      ] :
      strcontains(query, "percentile(`${local.metrics.agent_turn}`, 95)") &&
      strcontains(query, "WHERE ${local.operation_filter.agent_turn}") &&
      strcontains(query, "FACET `${local.attr.operation}`, `${local.attr.agent_context}`")
    ])

    error_message = "Agent turn P95 dashboard widget must keep percentile, WHERE operation filter, and operation x context FACET."
  }
}
