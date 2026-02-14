import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, GitBranch, Loader2, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import type { BranchCard as BranchCardType } from "@/types/git";
import type { AgentState } from "@/types/protocol";

function formatElapsed(timestampSec: number): string {
	const now = Date.now() / 1000;
	const diff = Math.max(0, Math.floor(now - timestampSec));
	if (diff < 60) return `${diff}s`;
	if (diff < 3600) return `${Math.floor(diff / 60)}m`;
	return `${Math.floor(diff / 3600)}h`;
}

const agentStateConfig: Record<
	AgentState,
	{ bg: string; text: string; dot: string; label: string }
> = {
	running: {
		bg: "bg-blue-500/15",
		text: "text-blue-500",
		dot: "bg-blue-500 animate-pulse",
		label: "Running",
	},
	done: {
		bg: "bg-green-500/15",
		text: "text-green-500",
		dot: "bg-green-500",
		label: "Done",
	},
	waiting: {
		bg: "bg-yellow-500/15",
		text: "text-yellow-500",
		dot: "bg-yellow-500 animate-pulse",
		label: "Waiting",
	},
	error: {
		bg: "bg-red-500/15",
		text: "text-red-500",
		dot: "bg-red-500",
		label: "Error",
	},
};

export function AgentStateBadge({
	state,
	timestamp,
}: {
	state: AgentState;
	timestamp?: number;
}) {
	const [, setTick] = useState(0);
	const config = agentStateConfig[state];

	useEffect(() => {
		if (!timestamp) return;
		const id = setInterval(() => setTick((t) => t + 1), 10000);
		return () => clearInterval(id);
	}, [timestamp]);

	return (
		<span className="shrink-0 inline-flex items-center gap-1">
			<span
				className={`inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-medium ${config.bg} ${config.text}`}
			>
				<span className={`w-1.5 h-1.5 rounded-full ${config.dot}`} />
				{config.label}
			</span>
			{timestamp && (
				<span className="text-[10px] text-muted-foreground">
					{formatElapsed(timestamp)}
				</span>
			)}
		</span>
	);
}

interface BranchCardProps {
	branch: BranchCardType;
	opening?: boolean;
	onOpen: () => void;
	onDelete: (branch: BranchCardType) => void;
}

export function BranchCard({
	branch,
	opening,
	onOpen,
	onDelete,
}: BranchCardProps) {
	const hasWorktree = branch.worktree_path != null;

	return (
		<div
			data-testid={`branch-card-${branch.name}`}
			className={`flex flex-col gap-3 rounded-lg border p-4 transition-colors ${
				hasWorktree
					? "border-border bg-card hover:border-primary/50"
					: "border-border/50 bg-card/50 hover:border-border"
			}`}
		>
			<div className="flex items-center gap-2 min-w-0">
				<GitBranch
					className={`size-4 shrink-0 ${hasWorktree ? "text-muted-foreground" : "text-muted-foreground/50"}`}
				/>
				<span
					className={`text-sm font-medium truncate ${!hasWorktree ? "text-muted-foreground" : ""}`}
				>
					{branch.name}
				</span>
				{branch.has_pr && branch.pr_url && branch.pr_number != null && (
					<button
						type="button"
						className="shrink-0 inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded bg-purple-500/15 text-purple-500 font-medium hover:bg-purple-500/25 transition-colors"
						onClick={(e) => {
							e.stopPropagation();
							openUrl(branch.pr_url as string);
						}}
					>
						PR #{branch.pr_number}
						<ExternalLink className="size-2.5" />
					</button>
				)}
				{branch.is_merged && (
					<span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-green-500/15 text-green-500 font-medium">
						merged
					</span>
				)}
				{branch.agent_state && (
					<AgentStateBadge
						state={branch.agent_state}
						timestamp={branch.agent_state_timestamp}
					/>
				)}
			</div>

			{hasWorktree && (
				<div className="text-xs">
					{branch.dirty_count > 0 ? (
						<span className="text-yellow-500">
							{branch.dirty_count} files changed
						</span>
					) : (
						<span className="text-green-500">clean</span>
					)}
				</div>
			)}

			<div className="flex items-center gap-2 mt-auto">
				<Button
					size="sm"
					className="flex-1"
					onClick={onOpen}
					disabled={opening}
				>
					{opening ? <Loader2 className="size-4 animate-spin" /> : "Open"}
				</Button>
				{(hasWorktree || branch.is_merged) && (
					<Button
						size="icon-sm"
						variant="ghost"
						onClick={() => onDelete(branch)}
						aria-label={
							hasWorktree
								? `Delete worktree for ${branch.name}`
								: `Delete branch ${branch.name}`
						}
					>
						<Trash2 className="size-4 text-muted-foreground" />
					</Button>
				)}
			</div>
		</div>
	);
}
