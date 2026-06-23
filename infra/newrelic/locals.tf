locals {
  service_name = "releash"

  metrics = {
    startup        = "releash.startup.duration_ms"
    hot_path       = "releash.hot_path.duration_ms"
    session_bytes  = "releash.session.save_bytes"
    payload_bytes  = "releash.agent_stream.payload_bytes"
    emit_interval  = "releash.agent_stream.emit_interval_ms"
    dropped_frames = "releash.agent_stream.dropped_frames"
    ws_reconnects  = "releash.agent_stream.ws_reconnects"
    rss_bytes      = "releash.process.rss_bytes"
    cpu_percent    = "releash.process.cpu_percent"
    xterm_count    = "releash.frontend.mounted_xterm_count"
    pty_count      = "releash.pty.active_count"
    op_status      = "releash.operation.status"
    usage_events   = "releash.usage.events"
  }

  attr = {
    operation   = "releash.operation"
    status      = "releash.status"
    channel     = "releash.channel"
    usage_event = "releash.usage_event"
  }

  allowed_resource_attrs = [
    "service.version",
    "os.type",
    "releash.build_type",
    "service.name",
  ]

  startup_operations = [
    "startup.app",
    "startup.first_window_ready",
    "startup.first_repo_snapshot_ready",
  ]

  repo_snapshot_operations = [
    "git.status_scan",
  ]

  diff_open_operations = [
    "git.diff_stats",
    "review.file_open",
  ]

  session_io_operations = [
    "session.list",
    "session.get_meta",
    "session.get_page",
    "session.load_full",
    "session.append",
    "session.persist_parts",
    "session.save_full",
  ]

  stream_channels = [
    "tauri_event",
    "websocket",
  ]

  usage_events = [
    "settings_saved",
    "worktree_created",
    "worktree_removed",
  ]

  budget = {
    repo_snapshot_p95_ms = 300
    diff_open_p95_ms     = 500
    payload_bytes        = 65536
  }

  alert_counts = {
    crash_window_min           = 5
    crash_threshold            = 5
    quality_window_min         = 5
    ws_reconnects_threshold    = 5
    dropped_frames_threshold   = 10
    sustained_window_seconds   = 300
    aggregation_window_seconds = 60
    aggregation_delay_seconds  = 120
  }

  violation_time_limit_seconds = 86400

  operation_filter = {
    startup       = "`${local.attr.operation}` IN (${join(", ", [for op in local.startup_operations : "'${op}'"])})"
    repo_snapshot = "`${local.attr.operation}` IN (${join(", ", [for op in local.repo_snapshot_operations : "'${op}'"])})"
    diff_open     = "`${local.attr.operation}` IN (${join(", ", [for op in local.diff_open_operations : "'${op}'"])})"
    session_io    = "`${local.attr.operation}` IN (${join(", ", [for op in local.session_io_operations : "'${op}'"])})"
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
