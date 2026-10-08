import { invokeClient as invoke } from "@/lib/client";
import type { WorkspaceNodeDetail } from "@/types/workspace-tree";
import { useStateSubscriptionResult } from "./useStateSubscription";

export interface WorkspaceNodeDetailState {
	detail: WorkspaceNodeDetail | null;
	loading: boolean;
	error: string | null;
	missingNodeId: string | null;
}

export function useWorkspaceNodeDetail({
	worktreePath,
	nodeId,
}: {
	worktreePath: string | null;
	nodeId: string | null;
}): WorkspaceNodeDetailState {
	const subscription = useStateSubscriptionResult(
		worktreePath && nodeId
			? { kind: "node-detail", args: [worktreePath, nodeId] }
			: null,
	);
	const detail = subscription.value;
	return {
		detail: detail ?? null,
		loading: Boolean(
			worktreePath && nodeId && detail === undefined && !subscription.error,
		),
		error: subscription.error,
		missingNodeId: detail === null ? nodeId : null,
	};
}

export async function approveWorkspaceNode({
	worktreePath,
	nodeId,
}: {
	worktreePath: string;
	nodeId: string;
}): Promise<void> {
	await invoke("approve_workspace_node", { worktreePath, nodeId });
}

export async function retryWorkspaceNode({
	worktreePath,
	nodeId,
}: {
	worktreePath: string;
	nodeId: string;
}): Promise<void> {
	await invoke("retry_workspace_node", { worktreePath, nodeId });
}

export async function resumeWorkspaceSessionNode({
	worktreePath,
	nodeId,
}: {
	worktreePath: string;
	nodeId: string;
}): Promise<void> {
	await invoke("resume_workspace_session_node", { worktreePath, nodeId });
}
