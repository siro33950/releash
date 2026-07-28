# Workspace tree test migration ledger

## Audit scope

The requested merge-base was audited with:

```bash
git show 76c1bb4fd55ead6ebaeb27843ec6bf139fca0771:src-tauri/src/usecase/workflow/workspace_tree.rs \
  | grep -Ec '#\[(tokio::)?test'
```

That exact revision contains 28 test attributes, not 44. Its `#[cfg(test)]`
module contains 37 functions when the nine fixture/helper functions are also
counted. The table below classifies all 28 tests found at the requested
revision, so the migration inventory does not silently omit a test because of
the count discrepancy.

Status meanings:

- **Restored**: the old behavior has a test on the current execution path.
- **Replaced**: a current test covers the same invariant at its new owner.
- **Retired**: the old mechanism no longer exists; the reason and the
  fail-closed/indexed replacement are recorded.

## Classification

| Merge-base test | Status | Current owner / reason |
| --- | --- | --- |
| `opaque_workflow_node_id_routes_to_one_execution_without_exposing_attempts` | Replaced | `domain/workspace_tree/entities/mod.rs::opaque_identity_digest_matches_sha256_and_normalizes_uuid` fixes opaque identity generation. Routing no longer parses an execution ID from the opaque ID; `local_event_store/tests.rs::b002_workspace_tree_node_and_session_binding_ignore_unrelated_accumulation` verifies the workspace-scoped indexed lookup. |
| `id_based_projection_targets_restrict_workflow_replay` | Retired | Query-time workflow replay and `WorkspaceProjectionTarget` were deleted. B-002 now verifies point/range SQL plans and that public queries do not scan event history. |
| `direct_session_is_a_leaf_and_summary_is_an_allowlist` | Replaced | `workspace_tree/repository.rs::direct_session_tree_distinguishes_empty_and_single_node_records` fixes the direct-Session leaf shape; `local_event_store/tests.rs::b001_workspace_tree_query_returns_complete_display_snapshot_once` verifies the public DTO projection. |
| `direct_session_close_target_is_materialized_only_for_its_opaque_node` | Restored | `workspace_tree/query_service.rs::direct_session_close_target_is_materialized_only_for_its_opaque_node`. |
| `error_reason_is_exposed_on_direct_session_badges_and_detail` | Restored | `workspace_tree/query_service.rs::error_reason_is_exposed_on_direct_session_badges_and_detail`. |
| `absent_definition_still_projects_actual_occurrences_without_queued_definitions` | Retired | The indexed node record is now the occurrence authority. Query code never needs a definition or replay fallback. Evolution fails closed when the required start/occurrence facts cannot produce a valid record. |
| `workflow_without_started_nodes_has_an_empty_branch_and_no_preferred_node` | Restored | `domain/workspace_tree/entities/mod.rs::workflow_without_started_nodes_has_an_empty_branch_and_no_preferred_node`. |
| `execution_occurrences_follow_event_order_without_unstarted_definitions` | Replaced | `domain/workspace_tree/entities/mod.rs::workspace_tree_projector_owns_parentage_identity_and_occurrence_order`. |
| `terminal_workflows_hide_every_unstarted_leaf_and_branch` | Replaced | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `started_nodes_keep_every_execution_status` | Restored | Same-named test in `domain/workspace_tree/projection.rs`, including the distinct aborted status. |
| `node_started_is_identical_in_live_and_reloaded_workspace_trees` | Replaced | `local_event_store/tests.rs::b005_live_restart_and_v2_evolution_preserve_workspace_tree` compares live, restart, and evolution results from the durable records. |
| `definition_only_nodes_never_populate_tree_detail_or_session_indexes` | Replaced | The domain test for an unstarted workflow verifies that definition-only rule records are not public nodes or preferred leaves; B-002 verifies detail and Session binding are indexed point lookups over materialized occurrences. |
| `fanout_occurrences_are_distinct_and_children_stay_nested_in_event_order` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `literal_fanout_projects_only_started_children_in_event_order` | Replaced | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `artifact_item_fanout_without_started_children_has_an_empty_branch` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`, including later materialization of distinct dynamic children. |
| `branch_status_capabilities_and_session_activity_are_backend_aggregated` | Restored | Same-named domain test verifies workflow capabilities, Session activity override, failed fanout aggregation, and waiting approval. |
| `resumable_workflow_exposes_the_durable_recovery_block_reason` | Replaced | `domain/workspace_tree/entities/mod.rs::workflow_recovery_reason_is_derived_from_stable_owner_order` and `local_event_store/tests.rs::unresolved_recovery_fence_survives_commit_restart_and_tree_read`. |
| `command_detail_contains_only_masked_snapshot_and_standard_result` | Replaced | `domain/workspace_tree/projection.rs::runtime_snapshot_nodes_uses_bounded_defaults_and_filters_other_executions` and `local_event_store/indexed_projection_codec.rs::tree_record_does_not_retain_command_detail_payloads`. |
| `command_detail_remains_bound_to_the_selected_occurrence` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `snapshot_mode_never_materializes_command_payloads` | Restored | Same-named co-located repository test proves the tree SELECT excludes `detail_record`; the codec test separately proves the tree record contains no command payload. |
| `each_occurrence_keeps_its_detail_and_only_waiting_occurrence_can_approve` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `missing_session_keeps_node_but_returns_no_unusable_session_id` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `stored_workflow_session_is_available_to_detail_and_opaque_lookup` | Replaced | `domain/workspace_tree/entities/mod.rs::closed_direct_session_leaves_tree_but_closed_workflow_session_keeps_structure` verifies the binding survives as workflow structure; B-002 verifies detail and opaque Session lookup. |
| `repeated_session_occurrences_keep_distinct_session_detail_and_lookup` | Restored | Same-named test in `domain/workspace_tree/entities/mod.rs`. |
| `selection_reconciliation_keeps_a_leaf_that_remains_in_the_snapshot` | Replaced | `usecase/workflow/workspace_tree.rs::selection_reconciliation_preserves_a_nested_node_in_the_new_snapshot`. |
| `archived_selection_reconciliation_uses_the_same_snapshot_preferred_leaf_or_null` | Replaced | `usecase/workflow/workspace_tree.rs::selection_reconciliation_reports_a_removed_node_without_replacing_selection`; reconciliation is now source-agnostic and archive visibility is applied before it. |
| `archived_workflow_is_hidden_from_tree_but_selected_session_detail_remains_available` | Replaced | `local_event_store/tests.rs::archived_workflow_is_hidden_while_selected_detail_remains_queryable`. |
| `failure_metadata_never_enters_workspace_summary_or_detail` | Restored | Same-named test in `domain/workspace_tree/projection.rs`; the projector maps internal failure metadata to a bounded public reason. |

