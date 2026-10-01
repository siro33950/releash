import type {
	WorkspaceTreeSnapshot as WireWorkspaceTreeSnapshot,
	WorkspaceListSnapshot,
} from "@/generated/client_types";
import type { WorkspaceTreeSnapshot } from "@/types/workspace-tree";

export function workspaceListSnapshot(
	snapshot: WorkspaceTreeSnapshot = {
		nodes: [],
		archivedSessions: [],
		preferredNodeId: null,
	},
	worktreePath = "/repo",
): WorkspaceListSnapshot {
	return {
		status: { loaded: true, error: null, state: "ready" },
		repositories: [
			{
				path: "/repo",
				status: { loaded: true, error: null, state: "ready" },
				branches: [
					{
						name: "feature",
						worktree_path: worktreePath,
						is_main_worktree: false,
						is_deleting: false,
						dirty_count: 0,
						dirty_count_error: null,
						pull_request_error: null,
						is_merged: false,
						has_pr: false,
						pr_number: null,
						pr_url: null,
					},
				],
				worktrees: [
					{
						path: worktreePath,
						status: {
							loaded: true,
							error: null,
							state: snapshot.nodes.length === 0 ? "empty" : "ready",
						},
						snapshot: snapshot as WireWorkspaceTreeSnapshot,
						workflowHistory: [],
					},
				],
			},
		],
	};
}
