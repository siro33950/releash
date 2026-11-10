resource "newrelic_one_dashboard" "releash" {
  name        = "Releash observability"
  permissions = var.dashboard_permissions

  page {
    name        = "Performance and reliability"
    description = "Releash server hot paths, resource pressure, and usage signals."

    widget_line {
      title  = "Startup P95 by operation"
      row    = 1
      column = 1
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.startup}`, 95) WHERE ${local.operation_filter.startup} FACET `${local.attr.operation}` TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
      units {
        unit = "ms"
      }
    }

    widget_line {
      title            = "Repo snapshot P95"
      row              = 1
      column           = 7
      width            = 6
      height           = 3
      is_label_visible = true

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE ${local.operation_filter.repo_snapshot} FACET `${local.attr.operation}` TIMESERIES"
      }

      threshold {
        name     = "M0 budget 300ms"
        from     = local.budget.repo_snapshot_p95_ms
        to       = 100000
        severity = "critical"
      }

      legend_enabled   = true
      y_axis_left_zero = true
      units {
        unit = "ms"
      }
    }

    widget_line {
      title  = "Process resources"
      row    = 7
      column = 1
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT average(`${local.metrics.rss_bytes}`) AS 'rss bytes' TIMESERIES"
      }

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT average(`${local.metrics.cpu_percent}`) AS 'cpu percent' TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
    }

    widget_line {
      title  = "Terminal pressure"
      row    = 7
      column = 7
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT average(`${local.metrics.pty_count}`) AS 'active ptys' TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
    }

    widget_bar {
      title  = "Operation status"
      row    = 10
      column = 1
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT sum(`${local.metrics.op_status}`) FACET `${local.attr.status}`"
      }
    }
  }
}
