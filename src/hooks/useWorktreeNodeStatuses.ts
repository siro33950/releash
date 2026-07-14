import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type {
	WorkspaceNodeStatus,
	WorktreeNodeStatusView,
} from "@/types/workspace-tree";

export interface WorktreeNodeStatuses {
	nodes: Map<string, WorkspaceNodeStatus>;
	executions: Map<string, WorkspaceNodeStatus>;
}

function viewToStatuses(view: WorktreeNodeStatusView): WorktreeNodeStatuses {
	const nodes = new Map<string, WorkspaceNodeStatus>();
	for (const node of view.nodeExecutions) {
		nodes.set(node.nodeExecutionId, node.representative);
	}
	const executions = new Map<string, WorkspaceNodeStatus>();
	for (const execution of view.workflowExecutions) {
		executions.set(execution.executionId, execution.representative);
	}
	return { nodes, executions };
}

export function useWorktreeNodeStatuses(
	worktreePath: string | null,
): WorktreeNodeStatuses {
	const [statuses, setStatuses] = useState<WorktreeNodeStatuses>({
		nodes: new Map(),
		executions: new Map(),
	});

	useEffect(() => {
		if (!worktreePath) {
			setStatuses({ nodes: new Map(), executions: new Map() });
			return;
		}

		let mounted = true;
		let unlisten: UnlistenFn | null = null;
		setStatuses({ nodes: new Map(), executions: new Map() });

		const applyView = (view: WorktreeNodeStatusView) => {
			if (view.worktreePath !== worktreePath) return;
			setStatuses(viewToStatuses(view));
		};

		const subscribe = async () => {
			let subscribed = false;
			try {
				unlisten = await listen<WorktreeNodeStatusView>(
					"workflow-node-status-changed",
					(event) => {
						if (!mounted) return;
						applyView(event.payload);
					},
				);
				subscribed = true;

				if (!mounted) {
					unlisten?.();
					return;
				}

				await invoke("sync_worktree_node_statuses", { worktreePath });
			} catch {
				if (mounted && !subscribed) {
					setStatuses({ nodes: new Map(), executions: new Map() });
				}
			}
		};

		void subscribe();

		return () => {
			mounted = false;
			unlisten?.();
		};
	}, [worktreePath]);

	return statuses;
}
