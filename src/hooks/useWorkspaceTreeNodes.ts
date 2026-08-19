import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { subscribeAgentSessionChanged } from "@/lib/agentSessionEvents";
import type { AgentSessionItem } from "@/types/agent-session";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
import type {
	WorkspaceTreeItem,
	WorkspaceTreeSelectionSnapshot,
	WorkspaceTreeSnapshot,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";

export interface WorkspaceReconciliationRequestContext {
	worktreePath: string;
	selectedNodeId: string;
	reconciliationGeneration: number;
}

export interface WorkspaceTreeReconciliationEvent {
	refreshSeq: number;
	requestContext: WorkspaceReconciliationRequestContext;
	selectionInSnapshot: boolean;
}

export interface WorkspaceTreeRefreshResult {
	snapshot: WorkspaceTreeSnapshot;
	reconciliationEvent: WorkspaceTreeReconciliationEvent | null;
}

interface WorkspaceTreeState {
	snapshot: WorkspaceTreeSnapshot;
	workflowHistory: WorkspaceWorkflowHistoryItem[];
	reconciliationEvent: WorkspaceTreeReconciliationEvent | null;
}

interface UseWorkspaceTreeNodesResult {
	nodes: WorkspaceTreeItem[];
	sessions: AgentSessionItem[];
	preferredNodeId: string | null;
	workflowHistory: WorkspaceWorkflowHistoryItem[];
	reconciliationEvent: WorkspaceTreeReconciliationEvent | null;
	loading: boolean;
	error: string | null;
	refresh: () => Promise<WorkspaceTreeRefreshResult | null>;
	beginArchiveReconciliation: (
		selectedNodeId: string,
	) => Promise<WorkspaceTreeRefreshResult | null>;
	synchronizeSelectedNodeId: (selectedNodeId: string | null) => void;
	isReconciliationEventCurrent: (
		event: WorkspaceTreeReconciliationEvent,
		selectedNodeId: string | null,
	) => boolean;
}

interface WorkspaceTreeRefreshDetail {
	worktreePath?: string;
}

const EMPTY_SNAPSHOT: WorkspaceTreeSnapshot = {
	nodes: [],
	sessions: [],
	preferredNodeId: null,
};

function sameReconciliationContext(
	left: WorkspaceReconciliationRequestContext | null,
	right: WorkspaceReconciliationRequestContext,
): boolean {
	return (
		left?.worktreePath === right.worktreePath &&
		left.selectedNodeId === right.selectedNodeId &&
		left.reconciliationGeneration === right.reconciliationGeneration
	);
}

export function useWorkspaceTreeNodes(
	worktreePath: string | null | undefined,
): UseWorkspaceTreeNodesResult {
	const [treeState, setTreeState] = useState<WorkspaceTreeState>({
		snapshot: EMPTY_SNAPSHOT,
		workflowHistory: [],
		reconciliationEvent: null,
	});
	const [loading, setLoading] = useState(() => Boolean(worktreePath));
	const [error, setError] = useState<string | null>(null);
	const refreshTimerRef = useRef<number | null>(null);
	const refreshSeqRef = useRef(0);
	const loadedWorktreePathRef = useRef<string | null>(null);
	const errorWorktreePathRef = useRef<string | null>(null);
	const worktreePathRef = useRef(worktreePath);
	const reconciliationGenerationRef = useRef(0);
	const reconciliationContextRef =
		useRef<WorkspaceReconciliationRequestContext | null>(null);
	const observedSelectedNodeIdRef = useRef<string | null>(null);
	const acceptedReconciliationSeqRef = useRef<number | null>(null);

	const hasLoadedCurrentWorktree = useCallback(
		() => loadedWorktreePathRef.current === worktreePath,
		[worktreePath],
	);

	const refresh = useCallback(async () => {
		if (!worktreePath) {
			refreshSeqRef.current += 1;
			loadedWorktreePathRef.current = null;
			errorWorktreePathRef.current = null;
			reconciliationContextRef.current = null;
			acceptedReconciliationSeqRef.current = null;
			setTreeState({
				snapshot: EMPTY_SNAPSHOT,
				workflowHistory: [],
				reconciliationEvent: null,
			});
			setLoading(false);
			setError(null);
			return null;
		}
		const seq = ++refreshSeqRef.current;
		const requestContext = reconciliationContextRef.current;
		const activeRequestContext =
			requestContext?.worktreePath === worktreePath &&
			requestContext.selectedNodeId === observedSelectedNodeIdRef.current
				? requestContext
				: null;
		const showLoading = !hasLoadedCurrentWorktree();
		if (showLoading) {
			setLoading(true);
		}
		try {
			const snapshotRequest = activeRequestContext
				? invoke<WorkspaceTreeSelectionSnapshot>(
						"get_workspace_tree_selection_reconciliation",
						{
							worktreePath,
							selectedNodeId: activeRequestContext.selectedNodeId,
						},
					)
				: invoke<WorkspaceTreeSnapshot>("list_workspace_worktree_nodes", {
						worktreePath,
					});
			const [treeResult, nextWorkflowHistory] = await Promise.all([
				snapshotRequest,
				invoke<WorkspaceWorkflowHistoryItem[]>(
					"list_workspace_workflow_history",
					{ worktreePath },
				),
			]);
			if (seq !== refreshSeqRef.current) return null;

			let snapshot: WorkspaceTreeSnapshot;
			let reconciliationEvent: WorkspaceTreeReconciliationEvent | null = null;
			if (activeRequestContext) {
				if (
					!sameReconciliationContext(
						reconciliationContextRef.current,
						activeRequestContext,
					) ||
					worktreePathRef.current !== activeRequestContext.worktreePath ||
					observedSelectedNodeIdRef.current !==
						activeRequestContext.selectedNodeId ||
					reconciliationGenerationRef.current !==
						activeRequestContext.reconciliationGeneration
				) {
					return null;
				}
				const selectionResult = treeResult as WorkspaceTreeSelectionSnapshot;
				snapshot = selectionResult.snapshot;
				reconciliationContextRef.current = null;
				acceptedReconciliationSeqRef.current = seq;
				reconciliationEvent = {
					refreshSeq: seq,
					requestContext: activeRequestContext,
					selectionInSnapshot:
						selectionResult.reconciliation.selectionInSnapshot,
				};
			} else {
				snapshot = treeResult as WorkspaceTreeSnapshot;
				acceptedReconciliationSeqRef.current = null;
			}

			loadedWorktreePathRef.current = worktreePath;
			errorWorktreePathRef.current = null;
			setTreeState({
				snapshot,
				workflowHistory: nextWorkflowHistory,
				reconciliationEvent,
			});
			setError(null);
			return { snapshot, reconciliationEvent };
		} catch (e) {
			if (seq !== refreshSeqRef.current) return null;
			if (!hasLoadedCurrentWorktree()) {
				setTreeState((current) => ({
					...current,
					snapshot: EMPTY_SNAPSHOT,
					workflowHistory: [],
				}));
			}
			errorWorktreePathRef.current = worktreePath;
			setError(String(e));
			return null;
		} finally {
			if (seq === refreshSeqRef.current) {
				setLoading(false);
			}
		}
	}, [hasLoadedCurrentWorktree, worktreePath]);

	const beginArchiveReconciliation = useCallback(
		(selectedNodeId: string) => {
			if (!worktreePath) return Promise.resolve(null);
			if (observedSelectedNodeIdRef.current !== selectedNodeId) {
				observedSelectedNodeIdRef.current = selectedNodeId;
				reconciliationGenerationRef.current += 1;
			}
			const requestContext = {
				worktreePath,
				selectedNodeId,
				reconciliationGeneration: ++reconciliationGenerationRef.current,
			};
			reconciliationContextRef.current = requestContext;
			acceptedReconciliationSeqRef.current = null;
			setTreeState((current) =>
				current.reconciliationEvent
					? { ...current, reconciliationEvent: null }
					: current,
			);
			return refresh();
		},
		[refresh, worktreePath],
	);

	const synchronizeSelectedNodeId = useCallback(
		(selectedNodeId: string | null) => {
			if (observedSelectedNodeIdRef.current === selectedNodeId) return;
			observedSelectedNodeIdRef.current = selectedNodeId;
			reconciliationGenerationRef.current += 1;
			reconciliationContextRef.current = null;
			acceptedReconciliationSeqRef.current = null;
			setTreeState((current) =>
				current.reconciliationEvent
					? { ...current, reconciliationEvent: null }
					: current,
			);
		},
		[],
	);

	const isReconciliationEventCurrent = useCallback(
		(event: WorkspaceTreeReconciliationEvent, selectedNodeId: string | null) =>
			event.refreshSeq === refreshSeqRef.current &&
			event.refreshSeq === acceptedReconciliationSeqRef.current &&
			event.requestContext.worktreePath === worktreePathRef.current &&
			event.requestContext.selectedNodeId === selectedNodeId &&
			event.requestContext.selectedNodeId ===
				observedSelectedNodeIdRef.current &&
			event.requestContext.reconciliationGeneration ===
				reconciliationGenerationRef.current,
		[],
	);

	const scheduleRefresh = useCallback(() => {
		if (refreshTimerRef.current != null) {
			window.clearTimeout(refreshTimerRef.current);
		}
		refreshTimerRef.current = window.setTimeout(() => {
			refreshTimerRef.current = null;
			void refresh();
		}, 80);
	}, [refresh]);

	useEffect(() => {
		if (worktreePathRef.current === worktreePath) return;
		worktreePathRef.current = worktreePath;
		refreshSeqRef.current += 1;
		reconciliationGenerationRef.current += 1;
		reconciliationContextRef.current = null;
		acceptedReconciliationSeqRef.current = null;
		observedSelectedNodeIdRef.current = null;
	}, [worktreePath]);

	useEffect(() => {
		void refresh();
	}, [refresh]);

	useEffect(() => {
		if (!worktreePath) return;
		let mounted = true;
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
		const unsubscribeAgentSessions = subscribeAgentSessionChanged(
			({ worktreePath: changedWorktreePath }) => {
				if (!mounted) return;
				if (changedWorktreePath && changedWorktreePath !== worktreePath) return;
				scheduleRefresh();
			},
		);

		const setup = async () => {
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
			unsubscribeAgentSessions();
			unlistenWorkflow?.();
			if (refreshTimerRef.current != null) {
				window.clearTimeout(refreshTimerRef.current);
				refreshTimerRef.current = null;
			}
		};
	}, [scheduleRefresh, worktreePath]);

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
		nodes: treeState.snapshot.nodes,
		sessions: treeState.snapshot.sessions ?? [],
		preferredNodeId: treeState.snapshot.preferredNodeId ?? null,
		workflowHistory: treeState.workflowHistory,
		reconciliationEvent: treeState.reconciliationEvent,
		loading: currentLoading,
		error: currentError,
		refresh,
		beginArchiveReconciliation,
		synchronizeSelectedNodeId,
		isReconciliationEventCurrent,
	};
}
