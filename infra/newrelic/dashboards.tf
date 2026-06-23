resource "newrelic_one_dashboard" "releash" {
  name        = "Releash observability"
  permissions = var.dashboard_permissions

  page {
    name        = "Performance and reliability"
    description = "Releash desktop app hot paths, agent streaming, resource pressure, and usage signals."

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
      title            = "Repo snapshot and session IO P95"
      row              = 1
      column           = 7
      width            = 6
      height           = 3
      is_label_visible = true

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE (${local.operation_filter.repo_snapshot} OR ${local.operation_filter.session_io}) FACET `${local.attr.operation}` TIMESERIES"
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
      title            = "Diff open P95"
      row              = 4
      column           = 1
      width            = 6
      height           = 3
      is_label_visible = true

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.hot_path}`, 95) WHERE ${local.operation_filter.diff_open} FACET `${local.attr.operation}` TIMESERIES"
      }

      threshold {
        name     = "M0 budget 500ms"
        from     = local.budget.diff_open_p95_ms
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
      title  = "Session save bytes P95"
      row    = 4
      column = 7
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.session_bytes}`, 95) WHERE ${local.operation_filter.session_io} FACET `${local.attr.operation}` TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
      units {
        unit = "bytes"
      }
    }

    widget_line {
      title            = "Agent stream payload P95"
      row              = 7
      column           = 1
      width            = 6
      height           = 3
      is_label_visible = true

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.payload_bytes}`, 95) WHERE `${local.attr.channel}` IN (${join(", ", [for channel in local.stream_channels : "'${channel}'"])}) FACET `${local.attr.channel}` TIMESERIES"
      }

      threshold {
        name     = "64KB frame budget"
        from     = local.budget.payload_bytes
        to       = 104857600
        severity = "critical"
      }

      legend_enabled   = true
      y_axis_left_zero = true
      units {
        unit = "bytes"
      }
    }

    widget_line {
      title  = "Agent stream quality"
      row    = 7
      column = 7
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT percentile(`${local.metrics.emit_interval}`, 95) AS 'emit interval p95 ms' TIMESERIES"
      }

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT sum(`${local.metrics.dropped_frames}`) AS 'dropped frames' TIMESERIES"
      }

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT sum(`${local.metrics.ws_reconnects}`) AS 'ws reconnects' TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
    }

    widget_line {
      title  = "Process resources"
      row    = 10
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
      row    = 10
      column = 7
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT average(`${local.metrics.xterm_count}`) AS 'mounted xterms' TIMESERIES"
      }

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT average(`${local.metrics.pty_count}`) AS 'active ptys' TIMESERIES"
      }

      legend_enabled   = true
      y_axis_left_zero = true
    }

    widget_bar {
      title  = "Operation status"
      row    = 13
      column = 1
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT sum(`${local.metrics.op_status}`) FACET `${local.attr.status}`"
      }
    }

    widget_bar {
      title  = "Usage events"
      row    = 13
      column = 7
      width  = 6
      height = 3

      nrql_query {
        account_id = var.newrelic_account_id
        query      = "FROM Metric SELECT sum(`${local.metrics.usage_events}`) WHERE `${local.attr.usage_event}` IN (${join(", ", [for event in local.usage_events : "'${event}'"])}) FACET `${local.attr.usage_event}`"
      }
    }
  }
}
