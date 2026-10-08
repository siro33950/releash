
CREATE TABLE IF NOT EXISTS logical_commits (
    commit_id TEXT PRIMARY KEY,
    installation_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL
        CHECK (operation_kind IN (
            'send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit',
            'recovery', 'user_mutation', 'shutdown_target',
            'operation_progress', 'projection', 'workflow'
        )),
    idempotency_key TEXT NOT NULL,
    payload_hash BLOB NOT NULL CHECK (length(payload_hash) = 32),
    state TEXT NOT NULL CHECK (state IN ('preparing', 'sealed')),
    first_global_sequence INTEGER CHECK (first_global_sequence IS NULL OR first_global_sequence >= 1),
    last_global_sequence INTEGER CHECK (last_global_sequence IS NULL OR last_global_sequence >= 1),
    event_count INTEGER NOT NULL CHECK (event_count >= 0),
    mutation_count INTEGER NOT NULL CHECK (mutation_count >= 0),
    stream_heads_json TEXT NOT NULL,
    result_hash BLOB CHECK (result_hash IS NULL OR length(result_hash) = 32),
    committed_at_ms INTEGER NOT NULL CHECK (committed_at_ms >= 0),
    UNIQUE (installation_id, operation_kind, idempotency_key),
    CHECK ((first_global_sequence IS NULL) = (last_global_sequence IS NULL))
);

CREATE TABLE IF NOT EXISTS stream_heads (
    stream_id TEXT PRIMARY KEY,
    head INTEGER NOT NULL CHECK (head >= 0),
    updated_commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS events (
    global_sequence INTEGER PRIMARY KEY CHECK (global_sequence >= 1),
    event_id TEXT NOT NULL UNIQUE,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    stream_id TEXT NOT NULL,
    stream_sequence INTEGER NOT NULL CHECK (stream_sequence >= 1),
    event_type TEXT NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version >= 1),
    occurred_at TEXT NOT NULL,
    payload BLOB NOT NULL,
    payload_sha256 BLOB NOT NULL CHECK (length(payload_sha256) = 32),
    UNIQUE (stream_id, stream_sequence)
);

CREATE TABLE IF NOT EXISTS operation_bindings (
    principal TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    caller_request_id TEXT NOT NULL,
    scope_id TEXT,
    operation_id TEXT NOT NULL,
    binding_hmac BLOB NOT NULL CHECK (length(binding_hmac) = 32),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (principal, installation_id, kind, caller_request_id)
);

CREATE TABLE IF NOT EXISTS caller_attempts (
    principal TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    caller_request_id TEXT NOT NULL,
    scope_id TEXT,
    command_hash BLOB NOT NULL CHECK (length(command_hash) = 32),
    sealed_command BLOB NOT NULL,
    resolution TEXT NOT NULL
        CHECK (resolution IN ('pending', 'accepted', 'rejected_before_commit', 'cleared')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (principal, installation_id, kind, caller_request_id)
);

CREATE TABLE IF NOT EXISTS operation_records (
    kind TEXT NOT NULL
        CHECK (kind IN ('send', 'permission_response', 'stop', 'session_lifecycle', 'application_quit')),
    operation_id TEXT NOT NULL,
    receipt TEXT NOT NULL,
    latest_status TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (kind, operation_id)
);

CREATE TABLE IF NOT EXISTS session_projection (
    session_id TEXT PRIMARY KEY,
    projection TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    workspace_identity TEXT,
    public_list_kind TEXT
        CHECK (public_list_kind IS NULL OR public_list_kind IN ('active', 'closed', 'archived')),
    public_sort_key_bits INTEGER,
    public_summary TEXT,
    CHECK (
        (public_list_kind IS NULL AND public_sort_key_bits IS NULL AND public_summary IS NULL)
        OR
        (workspace_identity IS NOT NULL AND public_list_kind IS NOT NULL
         AND public_sort_key_bits IS NOT NULL AND public_summary IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS obligations (
    obligation_id TEXT PRIMARY KEY,
    record TEXT NOT NULL,
    pending INTEGER NOT NULL CHECK (pending IN (0, 1)),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS pending_obligations (
    ordered_key TEXT PRIMARY KEY,
    obligation_id TEXT NOT NULL UNIQUE REFERENCES obligations (obligation_id),
    owner TEXT NOT NULL,
    partition TEXT NOT NULL
        CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
    shutdown_id TEXT,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS recovery_action_attempts (
    action_id TEXT PRIMARY KEY,
    binding_hash BLOB NOT NULL CHECK (length(binding_hash) = 32),
    attempt TEXT NOT NULL,
    completed TEXT,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS shutdown_plans (
    shutdown_id TEXT PRIMARY KEY,
    phase TEXT NOT NULL CHECK (phase IN (
        'prepared', 'activated', 'quiescing',
        'completed', 'failed', 'cancelled', 'reconciliation_required'
    )),
    summary TEXT NOT NULL,
    details_state TEXT NOT NULL CHECK (details_state IN ('available', 'compacted')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id)
);

CREATE TABLE IF NOT EXISTS shutdown_targets (
    shutdown_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (shutdown_id, ordinal),
    FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans (shutdown_id)
);

CREATE TABLE IF NOT EXISTS shutdown_recovery_snapshots (
    shutdown_id TEXT NOT NULL,
    partition TEXT NOT NULL
        CHECK (partition IN ('owner', 'closed_session', 'archived_session', 'unowned_runtime')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    detail TEXT NOT NULL,
    commit_id TEXT NOT NULL REFERENCES logical_commits (commit_id),
    PRIMARY KEY (shutdown_id, ordinal),
    FOREIGN KEY (shutdown_id) REFERENCES shutdown_plans (shutdown_id)
);

CREATE INDEX IF NOT EXISTS idx_pending_obligations_partition
    ON pending_obligations (partition, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_owner
    ON pending_obligations (owner, ordered_key);
CREATE INDEX IF NOT EXISTS idx_pending_obligations_shutdown
    ON pending_obligations (shutdown_id, ordered_key);
CREATE INDEX IF NOT EXISTS idx_shutdown_plans_details_state
    ON shutdown_plans (details_state);
CREATE INDEX IF NOT EXISTS idx_caller_attempts_scope
    ON caller_attempts (principal, installation_id, scope_id, kind, caller_request_id);
CREATE INDEX IF NOT EXISTS idx_caller_attempts_pending_kind
    ON caller_attempts (installation_id, kind, resolution, principal, caller_request_id);
CREATE INDEX IF NOT EXISTS idx_operation_bindings_operation
    ON operation_bindings (installation_id, kind, operation_id, principal, caller_request_id);
