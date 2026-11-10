locals {
  service_name = "releash"

  metrics = {
    startup     = "releash.startup.duration_ms"
    hot_path    = "releash.hot_path.duration_ms"
    rss_bytes   = "releash.process.rss_bytes"
    cpu_percent = "releash.process.cpu_percent"
    pty_count   = "releash.pty.active_count"
    op_status   = "releash.operation.status"
  }

  attr = {
    operation = "releash.operation"
    status    = "releash.status"
  }

  allowed_resource_attrs = [
    "service.version",
    "os.type",
    "releash.build_type",
    "service.name",
  ]

  startup_operations = [
    "startup.first_repo_snapshot_ready",
  ]

  repo_snapshot_operations = [
    "git.status_scan",
  ]


  budget = {
    repo_snapshot_p95_ms = 300
  }

  alert_counts = {
    crash_window_min           = 5
    crash_threshold            = 5
    sustained_window_seconds   = 300
    aggregation_window_seconds = 60
    aggregation_delay_seconds  = 120
  }

  violation_time_limit_seconds = 86400

  operation_filter = {
    startup       = "`${local.attr.operation}` IN (${join(", ", [for op in local.startup_operations : "'${op}'"])})"
    repo_snapshot = "`${local.attr.operation}` IN (${join(", ", [for op in local.repo_snapshot_operations : "'${op}'"])})"
  }

  pii_attribute_keys = [
    "device.id",
    "device_id",
    "deviceId",
    "session.id",
    "session_id",
    "sessionId",
    "user.id",
    "user_id",
    "userId",
    "user.email",
    "user.name",
    "email",
    "username",
    "path",
    "file.path",
    "file_path",
    "repo.path",
    "repo_path",
    "repository.path",
    "repository_path",
    "worktree.path",
    "worktree_path",
    "cwd",
    "current_dir",
    "directory",
    "home",
    "process.command_line",
  ]

  pii_delete_clause = join(", ", [
    for attr in local.pii_attribute_keys : strcontains(attr, ".") ? "`${attr}`" : attr
  ])

  metric_releash_scope = "(`service.name` = '${local.service_name}' OR metricName LIKE '${local.service_name}.%')"

  crash_error_filter = "(`service.name` = '${local.service_name}') AND (`event.name` = 'exception' OR eventName = 'exception' OR logger.name = 'releash.error' OR `exception.type` IS NOT NULL)"
}
