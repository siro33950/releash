import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { SessionStatus } from "@/types/session";
import type { WorkflowStatePayload } from "@/types/workflow";
import type { WorkspaceWorkflowStepDetail } from "@/types/workspace-tree";

interface UseWorkspaceWorkflowStepDetailInput {
	worktreePath: string | null;
	runId: string | null;
	stepId: string | null;
}

interface WorkspaceTreeRefreshDetail {
	worktreePath?: string;
}

export interface WorkspaceWorkflowStepDetailState {
	detail: WorkspaceWorkflowStepDetail | null;
	loading: boolean;
	error: string | null;
}

export function useWorkspaceWorkflowStepDetail({
	worktreePath,
	runId,
	stepId,
}: UseWorkspaceWorkflowStepDetailInput): WorkspaceWorkflowStepDetailState {
	const [state, setState] = useState<WorkspaceWorkflowStepDetailState>({
		detail: null,
		loading: false,
		error: null,
	});
	const detailRef = useRef<WorkspaceWorkflowStepDetail | null>(null);

	useEffect(() => {
		detailRef.current = state.detail;
	}, [state.detail]);

	useEffect(() => {
		if (!worktreePath || !runId || !stepId) {
			detailRef.current = null;
			setState({ detail: null, loading: false, error: null });
			return;
		}

		let cancelled = false;
		let unlistenWorkflow: UnlistenFn | null = null;
		let unlistenSessionStatus: UnlistenFn | null = null;
		detailRef.current = null;
		setState({ detail: null, loading: true, error: null });

		const load = (preserveDetail: boolean) => {
			setState((prev) => ({
				detail: preserveDetail ? prev.detail : null,
				loading: true,
				error: null,
			}));
			void invoke<WorkspaceWorkflowStepDetail | null>(
				"get_workspace_workflow_step_detail",
				{ worktreePath, runId, stepId },
			)
				.then((next) => {
					if (cancelled) return;
					if (next == null && preserveDetail && detailRef.current != null) {
						setState({
							detail: detailRef.current,
							loading: false,
							error: null,
						});
						return;
					}
					detailRef.current = next;
					setState({ detail: next, loading: false, error: null });
				})
				.catch((error) => {
					if (cancelled) return;
					const message =
						error instanceof Error ? error.message : String(error);
					if (preserveDetail && detailRef.current != null) {
						setState({
							detail: detailRef.current,
							loading: false,
							error: message,
						});
						return;
					}
					detailRef.current = null;
					setState({ detail: null, loading: false, error: message });
				});
		};

		const handleRefresh = (event: Event) => {
			const detail = (event as CustomEvent<WorkspaceTreeRefreshDetail>).detail;
			if (detail?.worktreePath && detail.worktreePath !== worktreePath) return;
			load(true);
		};
		load(false);
		window.addEventListener("workspace-tree-refresh", handleRefresh);

		const setup = async () => {
			const nextUnlistenWorkflow = await listen<WorkflowStatePayload>(
				"workflow-state-changed",
				(event) => {
					if (cancelled) return;
					if (event.payload.worktreePath !== worktreePath) return;
					if (event.payload.workflowState.executionId !== runId) return;
					load(true);
				},
			);
			if (cancelled) {
				nextUnlistenWorkflow();
				return;
			}
			unlistenWorkflow = nextUnlistenWorkflow;

			const nextUnlistenSessionStatus = await listen<SessionStatus>(
				"session-status-changed",
				(event) => {
					if (cancelled) return;
					if (event.payload.worktree_path !== worktreePath) return;
					const displayedStepSession = detailRef.current?.sessions.some(
						(session) => session.id === event.payload.chat_session_id,
					);
					const sessionWasHidden =
						event.payload.session_state === "closed" ||
						event.payload.session_state === "archived";
					if (!displayedStepSession && !sessionWasHidden) {
						return;
					}
					load(true);
				},
			);
			if (cancelled) {
				nextUnlistenSessionStatus();
				return;
			}
			unlistenSessionStatus = nextUnlistenSessionStatus;
		};

		void setup().catch(() => {});
		return () => {
			cancelled = true;
			window.removeEventListener("workspace-tree-refresh", handleRefresh);
			unlistenWorkflow?.();
			unlistenSessionStatus?.();
		};
	}, [runId, stepId, worktreePath]);

	return state;
}

export async function submitWorkspaceWorkflowStepAction({
	worktreePath,
	runId,
	stepId,
	stepName,
	nodeExecutionId,
}: {
	worktreePath: string;
	runId: string;
	stepId: string;
	stepName: string;
	nodeExecutionId?: string;
}): Promise<WorkspaceWorkflowStepDetail | null> {
	await invoke("approve_workflow_step", {
		args: {
			runId,
			stepName,
			nodeExecutionId: nodeExecutionId ?? null,
			comment: null,
		},
	});
	window.dispatchEvent(
		new CustomEvent("workspace-tree-refresh", { detail: { worktreePath } }),
	);
	return invoke<WorkspaceWorkflowStepDetail | null>(
		"get_workspace_workflow_step_detail",
		{ worktreePath, runId, stepId },
	);
}
