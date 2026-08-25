import { AlertTriangle } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useState } from "react";
import { type TogglePanel, ViewToolbar } from "@/components/layout/ViewToolbar";
import { AgentSessionRoute } from "@/components/panels/AgentSessionPanel";
import { Button } from "@/components/ui/button";
import { WorkflowNodeStatusIcon } from "@/components/workspace/WorkflowNodeStatusIcon";
import {
	approveWorkspaceNode,
	retryWorkspaceNode,
	useWorkspaceNodeDetail,
} from "@/hooks/useWorkspaceNodeDetail";
import { getErrorMessage } from "@/lib/errorMessage";
import type { AgentSessionLaunchAttachment } from "@/types/agent-session";
import type { Theme } from "@/types/settings";
import type {
	WorkspaceCommandNodeContent,
	WorkspaceNodeDetail,
} from "@/types/workspace-tree";

interface NodeContentViewProps {
	worktreePath: string;
	nodeId: string | null;
	theme?: Theme;
	leftPanels?: TogglePanel[];
	rightSlot?: React.ReactNode;
	onNodeMissing?: (worktreePath: string, nodeId: string) => void;
	initialSessionAttachment?: AgentSessionLaunchAttachment;
	onInitialSessionConsumed?: (agentSessionId: string) => void;
}

export function NodeContentView({
	worktreePath,
	nodeId,
	theme,
	leftPanels,
	rightSlot,
	onNodeMissing,
	initialSessionAttachment,
	onInitialSessionConsumed,
}: NodeContentViewProps) {
	const state = useWorkspaceNodeDetail({ worktreePath, nodeId });
	const detail = state.detail;

	useEffect(() => {
		if (!nodeId || state.missingNodeId !== nodeId) return;
		onNodeMissing?.(worktreePath, nodeId);
	}, [nodeId, onNodeMissing, state.missingNodeId, worktreePath]);

	return (
		<div className="flex h-full min-h-0 flex-col overflow-hidden">
			<ViewToolbar
				leftPanels={leftPanels}
				centerSlot={
					detail ? (
						<NodeHeader detail={detail} worktreePath={worktreePath} />
					) : null
				}
				rightSlot={rightSlot}
			/>
			<div className="flex min-h-0 flex-1 flex-col overflow-hidden">
				{detail ? (
					detail.content.kind === "session" ? (
						detail.content.sessionId ? (
							<AgentSessionRoute
								key={detail.content.sessionId}
								agentSessionId={detail.content.sessionId}
								theme={theme}
								initialAttachment={
									initialSessionAttachment?.agentSessionId ===
									detail.content.sessionId
										? initialSessionAttachment
										: undefined
								}
								onInitialSessionConsumed={onInitialSessionConsumed}
							/>
						) : (
							<NodeEmptyState message="Session unavailable." />
						)
					) : detail.content.kind === "command" ? (
						<CommandNodeContent detail={detail} content={detail.content} />
					) : (
						<NodeEmptyState message="Session unavailable." />
					)
				) : (
					<NodeEmptyState
						message={
							nodeId == null
								? "Select a Node from the Workspace tree."
								: state.loading
									? "Loading Node..."
									: "Node unavailable"
						}
						error={state.error}
					/>
				)}
			</div>
		</div>
	);
}

function NodeHeader({
	detail,
	worktreePath,
}: {
	detail: WorkspaceNodeDetail;
	worktreePath: string;
}) {
	const [approving, setApproving] = useState(false);
	const [retrying, setRetrying] = useState(false);
	const [actionError, setActionError] = useState<string | null>(null);

	const approve = useCallback(async () => {
		if (approving || !detail.capabilities.canApprove) return;
		setApproving(true);
		try {
			await approveWorkspaceNode({ worktreePath, nodeId: detail.id });
			setActionError(null);
		} catch (error) {
			setActionError(getErrorMessage(error));
		} finally {
			setApproving(false);
		}
	}, [approving, detail.capabilities.canApprove, detail.id, worktreePath]);

	const retry = useCallback(async () => {
		if (retrying || !detail.capabilities.canRetry) return;
		setRetrying(true);
		try {
			await retryWorkspaceNode({ worktreePath, nodeId: detail.id });
			setActionError(null);
		} catch (error) {
			setActionError(getErrorMessage(error));
		} finally {
			setRetrying(false);
		}
	}, [detail.capabilities.canRetry, detail.id, retrying, worktreePath]);

	const waitingMessage =
		detail.waitingFor === "stop"
			? "Submit received · waiting for Stop"
			: detail.waitingFor === "submit"
				? "Stop received · waiting for Submit"
				: null;
	const visibleErrorReason =
		detail.status === "failed" || detail.status === "paused"
			? detail.errorReason
			: null;

	return (
		<div className="flex min-w-0 items-center gap-2 pl-2">
			<span title={detail.status}>
				<WorkflowNodeStatusIcon
					status={detail.status}
					statusClassification={detail.statusClassification}
				/>
			</span>
			<span className="min-w-0 flex-1 truncate text-sm font-medium">
				{detail.title}
			</span>
			{waitingMessage && (
				<span className="min-w-0 truncate text-xs text-yellow-600 dark:text-yellow-300">
					{waitingMessage}
				</span>
			)}
			{detail.hasArtifact && (
				<span className="shrink-0 text-xs text-muted-foreground">
					Artifact submitted
				</span>
			)}
			{visibleErrorReason && (
				<span className="min-w-0 truncate text-xs text-destructive">
					{visibleErrorReason}
				</span>
			)}
			{detail.recoveryReason && (
				<span className="min-w-0 truncate text-xs text-orange-600 dark:text-orange-300">
					{detail.recoveryReason}
				</span>
			)}
			{actionError && (
				<span
					role="alert"
					className="flex min-w-0 items-center gap-1 truncate text-xs text-destructive"
					title={actionError}
				>
					<AlertTriangle className="size-3.5 shrink-0" />
					<span className="truncate">{actionError}</span>
				</span>
			)}
			{detail.capabilities.canApprove && (
				<Button type="button" size="xs" disabled={approving} onClick={approve}>
					{approving ? "Approving..." : "Approve"}
				</Button>
			)}
			{detail.capabilities.canRetry && (
				<Button type="button" size="xs" disabled={retrying} onClick={retry}>
					{retrying ? "Retrying..." : "Retry"}
				</Button>
			)}
		</div>
	);
}

function CommandNodeContent({
	detail,
	content,
}: {
	detail: WorkspaceNodeDetail;
	content: WorkspaceCommandNodeContent;
}) {
	return (
		<div className="h-full overflow-auto bg-background p-4">
			<div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
				<section className="space-y-2">
					<h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
						Command
					</h2>
					<pre
						data-testid="workspace-command"
						className="overflow-x-auto whitespace-pre-wrap break-words rounded-md border border-border bg-muted/30 p-3 text-sm"
					>
						{content.displayCommand ?? "Command has not been prepared yet."}
					</pre>
				</section>

				<section className="grid gap-3 text-sm sm:grid-cols-3">
					<CommandFact label="Status" value={detail.status} />
					<CommandFact
						label="Exit code"
						value={content.result ? String(content.result.exitCode) : "—"}
					/>
					<CommandFact
						label="Duration"
						value={content.result ? `${content.result.duration} ms` : "—"}
					/>
				</section>

				<CommandOutput label="stdout" value={content.result?.stdout ?? null} />
				<CommandOutput label="stderr" value={content.result?.stderr ?? null} />
			</div>
		</div>
	);
}

function CommandFact({ label, value }: { label: string; value: string }) {
	return (
		<div className="rounded-md border border-border p-3">
			<div className="text-xs text-muted-foreground">{label}</div>
			<div className="mt-1 font-mono">{value}</div>
		</div>
	);
}

function CommandOutput({
	label,
	value,
}: {
	label: "stdout" | "stderr";
	value: string | null;
}) {
	return (
		<section className="space-y-2">
			<h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
				{label}
			</h2>
			<pre
				data-testid={`workspace-command-${label}`}
				className="min-h-20 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-muted/30 p-3 font-mono text-xs"
			>
				{value ?? "No output."}
			</pre>
		</section>
	);
}

function NodeEmptyState({
	message,
	error,
}: {
	message: string;
	error?: string | null;
}) {
	return (
		<div className="flex h-full flex-col items-center justify-center gap-1 bg-background px-4 text-center text-sm text-muted-foreground">
			<div>{message}</div>
			{error && <div className="max-w-md break-words text-xs">{error}</div>}
		</div>
	);
}
