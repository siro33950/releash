import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { SessionStatus } from "@/types/session";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
import type { WorkspaceWorkflowNodeDetail } from "@/types/workspace-tree";

interface UseWorkspaceWorkflowNodeDetailInput {
	worktreePath: string | null;
	executionId: string | null;
	nodeExecutionId: string | null;
}

interface WorkspaceTreeRefreshDetail {
	worktreePath?: string;
}

export interface WorkspaceWorkflowNodeDetailState {
	detail: WorkspaceWorkflowNodeDetail | null;
	loading: boolean;
	error: string | null;
}

export function useWorkspaceWorkflowNodeDetail({
	worktreePath,
	executionId,
	nodeExecutionId,
}: UseWorkspaceWorkflowNodeDetailInput): WorkspaceWorkflowNodeDetailState {
	const [state, setState] = useState<WorkspaceWorkflowNodeDetailState>({
		detail: null,
		loading: false,
		error: null,
	});
	const detailRef = useRef<WorkspaceWorkflowNodeDetail | null>(null);

	useEffect(() => {
		detailRef.current = state.detail;
	}, [state.detail]);

	useEffect(() => {
		if (!worktreePath || !executionId || !nodeExecutionId) {
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
			void invoke<WorkspaceWorkflowNodeDetail | null>(
				"get_workspace_workflow_node_detail",
				{ worktreePath, executionId, nodeExecutionId },
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
			const nextUnlistenWorkflow =
				await listen<WorkflowExecutionChangedPayload>(
					"workflow-execution-changed",
					(event) => {
						if (cancelled) return;
						if (event.payload.worktreePath !== worktreePath) return;
						if (event.payload.workflowExecution.id !== executionId) return;
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
					const displayedNodeSession = detailRef.current?.sessions.some(
						(session) => session.id === event.payload.chat_session_id,
					);
					const sessionWasHidden =
						event.payload.session_state === "closed" ||
						event.payload.session_state === "archived";
					if (!displayedNodeSession && !sessionWasHidden) {
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
	}, [executionId, nodeExecutionId, worktreePath]);

	return state;
}

export async function submitWorkspaceWorkflowNodeAction({
	worktreePath,
	executionId,
	nodeExecutionId,
	nodeName,
}: {
	worktreePath: string;
	executionId: string;
	nodeExecutionId: string;
	nodeName: string;
}): Promise<WorkspaceWorkflowNodeDetail | null> {
	await invoke("approve_workflow_node", {
		args: {
			executionId,
			nodeName,
			nodeExecutionId,
			comment: null,
		},
	});
	window.dispatchEvent(
		new CustomEvent("workspace-tree-refresh", { detail: { worktreePath } }),
	);
	return invoke<WorkspaceWorkflowNodeDetail | null>(
		"get_workspace_workflow_node_detail",
		{ worktreePath, executionId, nodeExecutionId },
	);
}
