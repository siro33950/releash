import { useCallback, useContext, useEffect, useRef, useState } from "react";
import { subscribeState } from "@/lib/client";
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

export function useWorkspaceTreeNodes(worktreePath: string | null | undefined) {
	const model = useContext(WorkspaceListContext);
	if (!model) throw new Error("WorkspaceListContext is required");
	const list = model.snapshot?.repositories
		.flatMap((repo) => repo.worktrees)
		.find((tree) => tree.path === worktreePath);
	const [request, setRequest] =
		useState<WorkspaceReconciliationRequestContext | null>(null);
	const [reconciliationEvent, setEvent] =
		useState<WorkspaceTreeReconciliationEvent | null>(null);
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
				setEvent({
					refreshSeq: ++sequence.current,
					requestContext: request,
					selectionInSnapshot: value.reconciliation.selectionInSnapshot,
				});
			},
		);
	}, [request, worktreePath]);
	const beginArchiveReconciliation = useCallback(
		(selectedNodeId: string) => {
			if (!worktreePath) return;
			selected.current = selectedNodeId;
			setEvent(null);
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
		loading: Boolean(
			worktreePath && (!list || list.status.state === "loading"),
		),
		loaded: list?.status.loaded ?? false,
		state: list?.status.state ?? "loading",
		error: list?.status.error ?? null,
		beginArchiveReconciliation,
		synchronizeSelectedNodeId,
		isReconciliationEventCurrent,
	};
}
