import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type {
	WorkspaceTreeItem,
	WorkspaceTreeSnapshot,
	WorkspaceWorkflowHistoryItem,
} from "@/types/workspace-tree";
import { WorkspaceListContext } from "./useWorkspaceList";

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
	archivedSessions: WorkspaceTreeSnapshot["archivedSessions"];
	preferredNodeId: string | null;
	workflowHistory: WorkspaceWorkflowHistoryItem[];
	reconciliationEvent: WorkspaceTreeReconciliationEvent | null;
	loading: boolean;
	loaded: boolean;
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

const EMPTY_SNAPSHOT: WorkspaceTreeSnapshot = {
	nodes: [],
	archivedSessions: [],
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
	const workspaceList = useContext(WorkspaceListContext);
	if (!workspaceList) throw new Error("WorkspaceListContext is required");
	const refreshWorktree = workspaceList.refreshWorktree;
	const list = workspaceList.snapshot?.repositories
		.flatMap((repo) => repo.worktrees)
		.find((tree) => tree.path === worktreePath);
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
			if (!activeRequestContext) {
				const result = await refreshWorktree(worktreePath);
				if (seq !== refreshSeqRef.current || !result) return null;
				const tree = result.repositories
					.flatMap((repo) => repo.worktrees)
					.find((tree) => tree.path === worktreePath);
				return tree?.snapshot
					? { snapshot: tree.snapshot, reconciliationEvent: null }
					: null;
			}
			const selectionResult = await invoke(
				"get_workspace_tree_selection_reconciliation",
				{
					worktreePath,
					selectedNodeId: activeRequestContext.selectedNodeId,
				},
			);
			if (
				seq !== refreshSeqRef.current ||
				!sameReconciliationContext(
					reconciliationContextRef.current,
					activeRequestContext,
				) ||
				worktreePathRef.current !== activeRequestContext.worktreePath ||
				observedSelectedNodeIdRef.current !==
					activeRequestContext.selectedNodeId ||
				reconciliationGenerationRef.current !==
					activeRequestContext.reconciliationGeneration
			)
				return null;
			const snapshot = selectionResult.snapshot;
			reconciliationContextRef.current = null;
			acceptedReconciliationSeqRef.current = seq;
			const reconciliationEvent = {
				refreshSeq: seq,
				requestContext: activeRequestContext,
				selectionInSnapshot: selectionResult.reconciliation.selectionInSnapshot,
			};

			loadedWorktreePathRef.current = worktreePath;
			errorWorktreePathRef.current = null;
			setTreeState((current) => ({
				...current,
				snapshot,
				reconciliationEvent,
			}));
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
			setError(getErrorMessage(e));
			return null;
		} finally {
			if (seq === refreshSeqRef.current) {
				setLoading(false);
			}
		}
	}, [hasLoadedCurrentWorktree, worktreePath, refreshWorktree]);

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
		if (!worktreePath) {
			loadedWorktreePathRef.current = null;
			setTreeState({
				snapshot: EMPTY_SNAPSHOT,
				workflowHistory: [],
				reconciliationEvent: null,
			});
			setLoading(false);
			setError(null);
			return;
		}
		if (!list) {
			setTreeState({
				snapshot: EMPTY_SNAPSHOT,
				workflowHistory: [],
				reconciliationEvent: null,
			});
			return;
		}
		if (list.status.loaded) loadedWorktreePathRef.current = worktreePath;
		errorWorktreePathRef.current = list.status.error ? worktreePath : null;
		setError(list.status.error);
		setLoading(!list.status.loaded && !list.status.error);
		setTreeState((current) => ({
			...current,
			snapshot: list.snapshot ?? EMPTY_SNAPSHOT,
			workflowHistory: list.workflowHistory,
		}));
		if (reconciliationContextRef.current) void refresh();
	}, [list, refresh, worktreePath]);

	const currentError =
		(worktreePath ? workspaceList.worktreeErrors[worktreePath] : null) ??
		(errorWorktreePathRef.current === worktreePath ? error : null);
	const currentLoading =
		loading ||
		Boolean(
			worktreePath &&
				!hasLoadedCurrentWorktree() &&
				errorWorktreePathRef.current !== worktreePath,
		);

	return {
		nodes: treeState.snapshot.nodes,
		archivedSessions: treeState.snapshot.archivedSessions,
		preferredNodeId: treeState.snapshot.preferredNodeId ?? null,
		workflowHistory: treeState.workflowHistory,
		reconciliationEvent: treeState.reconciliationEvent,
		loading: !currentError && currentLoading,
		loaded: hasLoadedCurrentWorktree(),
		error: currentError,
		refresh,
		beginArchiveReconciliation,
		synchronizeSelectedNodeId,
		isReconciliationEventCurrent,
	};
}
