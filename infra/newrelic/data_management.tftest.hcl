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

