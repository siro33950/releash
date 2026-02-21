import { ArrowLeft } from "lucide-react";
import type { ConnectionStatus } from "../hooks/useWebSocket";
import { StatusIndicator } from "./StatusIndicator";

interface RemoteAppHeaderProps {
	selectedWorktree: string | null;
	branchName: string | null;
	status: ConnectionStatus;
	onBackToWorktrees: () => void;
	onDisconnect: () => void;
}

export function RemoteAppHeader({
	selectedWorktree,
	branchName,
	status,
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
			</div>
			<div className="flex items-center gap-2 shrink-0">
				<StatusIndicator status={status} />
				<button
					type="button"
					onClick={onDisconnect}
					className="text-xs px-2 py-0.5 rounded bg-secondary hover:bg-secondary/80 text-secondary-foreground transition-colors"
				>
					切断
				</button>
			</div>
		</header>
	);
}
