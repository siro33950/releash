import type {
	WorkspaceListSnapshotDto,
	WorkspaceTreeSnapshotDto,
} from "@/generated/client_types";
import type { WorkspaceTreeSnapshot } from "@/types/workspace-tree";

export function workspaceListSnapshot(
	snapshot: WorkspaceTreeSnapshot = {
		nodes: [],
		archivedSessions: [],
		preferredNodeId: null,
	},
	worktreePath = "/repo",
): WorkspaceListSnapshotDto {
	return {
		generation: 1,
		status: { loaded: true, error: null },
		repositories: [
			{
				path: "/repo",
				status: { loaded: true, error: null },
				branches: [
					{
						name: "feature",
						worktree_path: worktreePath,
						is_main_worktree: false,
						is_deleting: false,
						dirty_count: 0,
						is_merged: false,
						ahead: 0,
						behind: 0,
						has_upstream: false,
						base_ahead: 0,
						has_pr: false,
						pr_number: null,
						pr_url: null,
					},
				],
				worktrees: [
					{
						path: worktreePath,
						status: { loaded: true, error: null },
						snapshot: snapshot as WorkspaceTreeSnapshotDto,
						workflowHistory: [],
					},
				],
			},
		],
	};
}
