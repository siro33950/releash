import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { listClosedSessions } from "@/hooks/useSessionStore";
import type { SessionStatus } from "@/types/session";
import type { WorkflowStatePayload } from "@/types/workflow";
import type {
	WorkspaceSessionHistoryItem,
	WorkspaceTreeNode,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";

interface UseWorkspaceTreeNodesResult {
	nodes: WorkspaceTreeNode[];
	closedSessions: WorkspaceSessionHistoryItem[];
	workflowHistory: WorkspaceWorkflowHistoryItem[];
	loading: boolean;
	error: string | null;
	refresh: () => Promise<void>;
}

interface WorkspaceTreeRefreshDetail {
	worktreePath?: string;
}

export function useWorkspaceTreeNodes(
	worktreePath: string | null | undefined,
): UseWorkspaceTreeNodesResult {
	const [nodes, setNodes] = useState<WorkspaceTreeNode[]>([]);
	const [closedSessions, setClosedSessions] = useState<
		WorkspaceSessionHistoryItem[]
	>([]);
	const [workflowHistory, setWorkflowHistory] = useState<
		WorkspaceWorkflowHistoryItem[]
	>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const refreshTimerRef = useRef<number | null>(null);
	const refreshSeqRef = useRef(0);
	const loadedWorktreePathRef = useRef<string | null>(null);
	const nodesRef = useRef<WorkspaceTreeNode[]>([]);

	const updateNodes = useCallback((nextNodes: WorkspaceTreeNode[]) => {
		nodesRef.current = nextNodes;
		setNodes(nextNodes);
	}, []);

	const hasLoadedCurrentWorktree = useCallback(
		() => loadedWorktreePathRef.current === worktreePath,
		[worktreePath],
	);

	const refresh = useCallback(async () => {
		if (!worktreePath) {
			refreshSeqRef.current += 1;
			loadedWorktreePathRef.current = null;
			nodesRef.current = [];
			setNodes([]);
			setClosedSessions([]);
			setWorkflowHistory([]);
			setLoading(false);
			setError(null);
			return;
		}
		const seq = ++refreshSeqRef.current;
		const showLoading = !hasLoadedCurrentWorktree();
		if (showLoading) {
			setLoading(true);
		}
		try {
			const nextNodes = await invoke<WorkspaceTreeNode[]>(
				"list_workspace_worktree_nodes",
				{ worktreePath },
			);
			const [nextClosedSessions, nextWorkflowHistory] = await Promise.all([
				listClosedSessions(worktreePath),
				invoke<WorkspaceWorkflowHistoryItem[]>(
					"list_workspace_workflow_history",
					{ worktreePath },
				),
			]);
			if (seq !== refreshSeqRef.current) return;
			loadedWorktreePathRef.current = worktreePath;
			updateNodes(nextNodes);
			setClosedSessions(
				nextClosedSessions.filter((session) => !session.workflowStepSession),
			);
			setWorkflowHistory(nextWorkflowHistory);
			setError(null);
		} catch (e) {
			if (seq !== refreshSeqRef.current) return;
			if (!hasLoadedCurrentWorktree()) {
				updateNodes([]);
				setClosedSessions([]);
				setWorkflowHistory([]);
			}
			setError(String(e));
		} finally {
			if (seq === refreshSeqRef.current) {
				setLoading(false);
			}
		}
	}, [hasLoadedCurrentWorktree, updateNodes, worktreePath]);

	const scheduleRefresh = useCallback(() => {
		if (refreshTimerRef.current != null) {
			window.clearTimeout(refreshTimerRef.current);
		}
		refreshTimerRef.current = window.setTimeout(() => {
			refreshTimerRef.current = null;
			void refresh();
		}, 80);
	}, [refresh]);

	const hasSessionNode = useCallback((sessionId: string): boolean => {
		return nodesRef.current.some((node) => {
			if (node.kind === "session") {
				return node.id === sessionId;
			}
			return node.children.some((child) => child.id === sessionId);
		});
	}, []);

	const hasWorkflowNode = useCallback((runId: string): boolean => {
		return nodesRef.current.some(
			(node) => node.kind === "workflow" && node.runId === runId,
		);
	}, []);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	useEffect(() => {
		if (!worktreePath) return;
		let mounted = true;
		let unlistenStatus: UnlistenFn | null = null;
		let unlistenWorkflow: UnlistenFn | null = null;

		const handleWorkspaceTreeRefresh = (event: Event) => {
			if (!mounted) return;
			const detail = (event as CustomEvent<WorkspaceTreeRefreshDetail>).detail;
			if (detail?.worktreePath && detail.worktreePath !== worktreePath) return;
			scheduleRefresh();
		};

		window.addEventListener(
			"workspace-tree-refresh",
			handleWorkspaceTreeRefresh,
		);

		const setup = async () => {
			unlistenStatus = await listen<SessionStatus>(
				"session-status-changed",
				(event) => {
					if (!mounted) return;
					if (event.payload.worktree_path !== worktreePath) return;
					if (
						!hasSessionNode(event.payload.chat_session_id) ||
						event.payload.session_state === "closed" ||
						event.payload.session_state === "archived"
					) {
						scheduleRefresh();
					}
				},
			);
			unlistenWorkflow = await listen<WorkflowStatePayload>(
				"workflow-state-changed",
				(event) => {
					if (!mounted) return;
					if (event.payload.worktreePath !== worktreePath) return;
					const stateType = event.payload.workflowState.state.type;
					if (
						!hasWorkflowNode(event.payload.workflowState.executionId) ||
						stateType === "completed" ||
						stateType === "failed" ||
						stateType === "aborted"
					) {
						scheduleRefresh();
					}
				},
			);
		};

		void setup().catch(() => {});
		return () => {
			mounted = false;
			window.removeEventListener(
				"workspace-tree-refresh",
				handleWorkspaceTreeRefresh,
			);
			unlistenStatus?.();
			unlistenWorkflow?.();
			if (refreshTimerRef.current != null) {
				window.clearTimeout(refreshTimerRef.current);
				refreshTimerRef.current = null;
			}
		};
	}, [hasSessionNode, hasWorkflowNode, scheduleRefresh, worktreePath]);

	return { nodes, closedSessions, workflowHistory, loading, error, refresh };
}
