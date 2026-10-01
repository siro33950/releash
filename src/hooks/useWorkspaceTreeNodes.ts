import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { subscribeState } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
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

export function useWorkspaceTreeNodes(worktreePath: string) {
	const model = useContext(WorkspaceListContext);
	if (!model) throw new Error("WorkspaceListContext is required");
	const list = model.snapshot?.repositories
		.flatMap((repo) => repo.worktrees)
		.find((tree) => tree.path === worktreePath);
	const [request, setRequest] =
		useState<WorkspaceReconciliationRequestContext | null>(null);
	const [reconciliationEvent, setEvent] =
		useState<WorkspaceTreeReconciliationEvent | null>(null);
	const [reconciliationError, setReconciliationError] = useState<string | null>(
		null,
	);
	const generation = useRef(0);
	const sequence = useRef(0);
	const selected = useRef<string | null>(null);
	useEffect(() => {
		if (!request || request.worktreePath !== worktreePath) return;
		return subscribeState(
			{
				kind: "selection",
				args: [request.worktreePath, request.selectedNodeId],
			},
			(value) => {
				if (request.reconciliationGeneration !== generation.current) return;
				setReconciliationError(null);
				setEvent({
					refreshSeq: ++sequence.current,
					requestContext: request,
					selectionInSnapshot: value.reconciliation.selectionInSnapshot,
				});
			},
			(error) => {
				if (request.reconciliationGeneration === generation.current)
					setReconciliationError(getErrorMessage(error));
			},
		);
	}, [request, worktreePath]);
	const beginArchiveReconciliation = useCallback(
		(selectedNodeId: string) => {
			selected.current = selectedNodeId;
			setEvent(null);
			setReconciliationError(null);
			setRequest({
				worktreePath,
				selectedNodeId,
				reconciliationGeneration: ++generation.current,
			});
		},
		[worktreePath],
	);
	const synchronizeSelectedNodeId = useCallback((nodeId: string | null) => {
		if (selected.current === nodeId) return;
		selected.current = nodeId;
		generation.current++;
		setRequest(null);
		setEvent(null);
		setReconciliationError(null);
	}, []);
	const isReconciliationEventCurrent = useCallback(
		(event: WorkspaceTreeReconciliationEvent, nodeId: string | null) =>
			event.refreshSeq === sequence.current &&
			event.requestContext.reconciliationGeneration === generation.current &&
			event.requestContext.worktreePath === worktreePath &&
			event.requestContext.selectedNodeId === nodeId &&
			selected.current === nodeId,
		[worktreePath],
	);
	return {
		nodes: list?.snapshot?.nodes ?? [],
		archivedSessions: list?.snapshot?.archivedSessions ?? [],
		preferredNodeId: list?.snapshot?.preferredNodeId ?? null,
		workflowHistory: list?.workflowHistory ?? [],
		reconciliationEvent,
		loading: !list || list.status.state === "loading",
		loaded: list?.status.loaded ?? false,
		state: list?.status.state ?? "loading",
		error:
			(request?.worktreePath === worktreePath ? reconciliationError : null) ??
			list?.status.error ??
			null,
		beginArchiveReconciliation,
		synchronizeSelectedNodeId,
		isReconciliationEventCurrent,
	};
}
