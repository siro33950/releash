import { ArrowLeft } from "lucide-react";
import type { WorkflowState } from "@/types/workflow";
import type { ConnectionStatus } from "../hooks/useWebSocket";
import { StatusIndicator } from "./StatusIndicator";

interface RemoteAppHeaderProps {
	selectedWorktree: string | null;
	branchName: string | null;
	status: ConnectionStatus;
	workflowState: WorkflowState | null;
	onBackToWorktrees: () => void;
	onDisconnect: () => void;
}

function workflowStateBadge(state: WorkflowState) {
	const label = `${state.workflowName}: ${state.currentStepName}`;
	const colors: Record<string, string> = {
		running: "bg-blue-500/20 text-blue-400",
		waiting_approval: "bg-yellow-500/20 text-yellow-400",
		completed: "bg-green-500/20 text-green-400",
		failed: "bg-red-500/20 text-red-400",
		aborted: "bg-gray-500/20 text-gray-400",
	};
	const color = colors[state.state.type] ?? "bg-gray-500/20 text-gray-400";
	return (
		<span className={`text-xs px-1.5 py-0.5 rounded ${color}`}>{label}</span>
	);
}

export function RemoteAppHeader({
	selectedWorktree,
	branchName,
	status,
	workflowState,
	onBackToWorktrees,
	onDisconnect,
}: RemoteAppHeaderProps) {
	return (
		<header className="flex items-center justify-between px-3 py-1.5 border-b border-border bg-card shrink-0">
			<div className="flex items-center gap-2 min-w-0">
				{selectedWorktree && (
					<button
						type="button"
						onClick={onBackToWorktrees}
						className="p-1 -ml-1 rounded hover:bg-muted transition-colors shrink-0"
						aria-label="Back"
					>
						<ArrowLeft className="size-4" />
					</button>
				)}
				<h1 className="text-sm font-semibold shrink-0">Releash Remote</h1>
				{branchName && (
					<span className="text-xs text-muted-foreground truncate font-mono">
						{branchName}
					</span>
				)}
				{workflowState && workflowStateBadge(workflowState)}
			</div>
			<div className="flex items-center gap-2 shrink-0">
				<StatusIndicator status={status} />
				<button
					type="button"
					onClick={onDisconnect}
					className="text-xs px-2 py-0.5 rounded bg-secondary hover:bg-secondary/80 text-secondary-foreground transition-colors"
				>
					Disconnect
				</button>
			</div>
		</header>
	);
}
