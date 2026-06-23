resource "newrelic_metric_pruning_rule" "pii_metric_aggregates" {
  account_id  = var.newrelic_account_id
  description = "Prunes high-cardinality PII/path attributes from #1209 Releash dimensional metric aggregates, the exported high-volume signal used for free-tier ingest control."
  nrql        = "SELECT ${local.pii_delete_clause} FROM Metric WHERE ${local.metric_releash_scope}"
}
