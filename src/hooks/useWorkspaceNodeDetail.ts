import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { SessionStatus } from "@/types/session";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
import type { WorkspaceNodeDetail } from "@/types/workspace-tree";

interface UseWorkspaceNodeDetailInput {
	worktreePath: string | null;
	nodeId: string | null;
}

interface WorkspaceTreeRefreshDetail {
	worktreePath?: string;
}

export interface WorkspaceNodeDetailState {
	detail: WorkspaceNodeDetail | null;
	loading: boolean;
	error: string | null;
	missingNodeId: string | null;
}

export function useWorkspaceNodeDetail({
	worktreePath,
	nodeId,
}: UseWorkspaceNodeDetailInput): WorkspaceNodeDetailState {
	const [state, setState] = useState<WorkspaceNodeDetailState>({
		detail: null,
		loading: false,
		error: null,
		missingNodeId: null,
	});
	const detailRef = useRef<WorkspaceNodeDetail | null>(null);
	const loadSeqRef = useRef(0);

	useEffect(() => {
		detailRef.current = state.detail;
	}, [state.detail]);

	useEffect(() => {
		if (!worktreePath || !nodeId) {
			loadSeqRef.current += 1;
			detailRef.current = null;
			setState({
				detail: null,
				loading: false,
				error: null,
				missingNodeId: null,
			});
			return;
		}

		let cancelled = false;
		let unlistenWorkflow: UnlistenFn | null = null;
		let unlistenSessionStatus: UnlistenFn | null = null;
		detailRef.current = null;
		setState({
			detail: null,
			loading: true,
			error: null,
			missingNodeId: null,
		});

		const load = (preserveDetail: boolean) => {
			const loadSeq = ++loadSeqRef.current;
			setState((previous) => ({
				detail: preserveDetail ? previous.detail : null,
				loading: true,
				error: null,
				missingNodeId: null,
			}));
			void invoke<WorkspaceNodeDetail | null>("get_workspace_node_detail", {
				worktreePath,
				nodeId,
			})
				.then((next) => {
					if (cancelled || loadSeq !== loadSeqRef.current) return;
					detailRef.current = next;
					setState({
						detail: next,
						loading: false,
						error: null,
						missingNodeId: next == null ? nodeId : null,
					});
				})
				.catch((error) => {
					if (cancelled || loadSeq !== loadSeqRef.current) return;
					const message =
						error instanceof Error ? error.message : String(error);
					if (preserveDetail && detailRef.current != null) {
						setState({
							detail: detailRef.current,
							loading: false,
							error: message,
							missingNodeId: null,
						});
						return;
					}
					detailRef.current = null;
					setState({
						detail: null,
						loading: false,
						error: message,
						missingNodeId: null,
					});
				});
		};

		const handleRefresh = (event: Event) => {
			const detail = (event as CustomEvent<WorkspaceTreeRefreshDetail>).detail;
			if (detail?.worktreePath && detail.worktreePath !== worktreePath) return;
			load(true);
		};

		window.addEventListener("workspace-tree-refresh", handleRefresh);

		const setup = async () => {
			const nextUnlistenWorkflow =
				await listen<WorkflowExecutionChangedPayload>(
					"workflow-execution-changed",
					(event) => {
						if (cancelled) return;
						if (event.payload.worktreePath !== worktreePath) return;
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
					load(true);
				},
			);
			if (cancelled) {
				nextUnlistenSessionStatus();
				return;
			}
			unlistenSessionStatus = nextUnlistenSessionStatus;
			load(false);
		};

		void setup().catch(() => {
			if (!cancelled) load(false);
		});
		return () => {
			cancelled = true;
			window.removeEventListener("workspace-tree-refresh", handleRefresh);
			unlistenWorkflow?.();
			unlistenSessionStatus?.();
		};
	}, [nodeId, worktreePath]);

	return state;
}

export async function approveWorkspaceNode({
	worktreePath,
	nodeId,
}: {
	worktreePath: string;
	nodeId: string;
}): Promise<WorkspaceNodeDetail | null> {
	await invoke("approve_workspace_node", { worktreePath, nodeId });
	window.dispatchEvent(
		new CustomEvent("workspace-tree-refresh", { detail: { worktreePath } }),
	);
	return invoke<WorkspaceNodeDetail | null>("get_workspace_node_detail", {
		worktreePath,
		nodeId,
	});
}

export async function retryWorkspaceNode({
	worktreePath,
	nodeId,
}: {
	worktreePath: string;
	nodeId: string;
}): Promise<WorkspaceNodeDetail | null> {
	await invoke("retry_workspace_node", { worktreePath, nodeId });
	window.dispatchEvent(
		new CustomEvent("workspace-tree-refresh", { detail: { worktreePath } }),
	);
	return invoke<WorkspaceNodeDetail | null>("get_workspace_node_detail", {
		worktreePath,
		nodeId,
	});
}
