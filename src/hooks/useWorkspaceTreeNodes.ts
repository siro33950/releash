import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { listClosedSessions } from "@/hooks/useSessionStore";
import type { SessionStatus } from "@/types/session";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
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
	const [loading, setLoading] = useState(() => Boolean(worktreePath));
	const [error, setError] = useState<string | null>(null);
	const refreshTimerRef = useRef<number | null>(null);
	const refreshSeqRef = useRef(0);
	const loadedWorktreePathRef = useRef<string | null>(null);
	const errorWorktreePathRef = useRef<string | null>(null);
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
			errorWorktreePathRef.current = null;
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
			const [nextNodes, nextClosedSessions, nextWorkflowHistory] =
				await Promise.all([
					invoke<WorkspaceTreeNode[]>("list_workspace_worktree_nodes", {
						worktreePath,
					}),
					listClosedSessions(worktreePath),
					invoke<WorkspaceWorkflowHistoryItem[]>(
						"list_workspace_workflow_history",
						{ worktreePath },
					),
				]);
			if (seq !== refreshSeqRef.current) return;
			loadedWorktreePathRef.current = worktreePath;
			errorWorktreePathRef.current = null;
			updateNodes(nextNodes);
			setClosedSessions(
				nextClosedSessions.filter((session) => !session.workflowNodeSession),
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
			errorWorktreePathRef.current = worktreePath;
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
			return node.nodeExecutions.some((execution) =>
				execution.sessions.some((session) => session.id === sessionId),
			);
		});
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
			const nextUnlistenStatus = await listen<SessionStatus>(
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
			if (!mounted) {
				nextUnlistenStatus();
				return;
			}
			unlistenStatus = nextUnlistenStatus;

			const nextUnlistenWorkflow =
				await listen<WorkflowExecutionChangedPayload>(
					"workflow-execution-changed",
					(event) => {
						if (!mounted) return;
						if (event.payload.worktreePath !== worktreePath) return;
						scheduleRefresh();
					},
				);
			if (!mounted) {
				nextUnlistenWorkflow();
				return;
			}
			unlistenWorkflow = nextUnlistenWorkflow;
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
	}, [hasSessionNode, scheduleRefresh, worktreePath]);

	const currentError =
		errorWorktreePathRef.current === worktreePath ? error : null;
	const currentLoading =
		loading ||
		Boolean(
			worktreePath &&
				!hasLoadedCurrentWorktree() &&
				errorWorktreePathRef.current !== worktreePath,
		);

	return {
		nodes,
		closedSessions,
		workflowHistory,
		loading: currentLoading,
		error: currentError,
		refresh,
	};
}
