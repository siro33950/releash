import {
	FolderGit2,
	GitBranch,
	GitPullRequest,
	Lock,
	RefreshCw,
} from "lucide-react";
import { useMemo } from "react";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import { aggregateAgentState } from "@/lib/agentStateUtils";
import type {
	AgentState,
	AgentStateSync,
	WorktreeEntryMsg,
} from "@/types/protocol";

interface RemoteDashboardProps {
	worktrees: WorktreeEntryMsg[];
	loading: boolean;
	onRefresh: () => void;
	onSelect?: (worktreePath: string) => void;
	agentStates?: Map<string, AgentStateSync>;
}

function repoDisplayName(repoPath: string): string {
	return repoPath.split("/").pop() ?? repoPath;
}

function WorktreeCard({
	wt,
	agentState,
	onSelect,
}: {
	wt: WorktreeEntryMsg;
	agentState?: AgentState;
	onSelect?: (path: string) => void;
}) {
	return (
		<button
			key={wt.path}
			type="button"
			className="flex flex-col gap-3 p-4 rounded-lg border border-border bg-card/50 hover:border-muted-foreground hover:bg-card transition-colors text-left w-full"
			onClick={() => onSelect?.(wt.path)}
		>
			<div className="flex items-center gap-2 min-w-0">
				<GitBranch className="size-4 shrink-0 text-muted-foreground" />
				<span className="text-sm font-medium truncate">{wt.branch}</span>
				{wt.is_locked && <Lock className="size-3 text-warning shrink-0" />}
				{wt.has_pr && (
					<span className="shrink-0 inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded bg-info/15 text-info font-medium">
						<GitPullRequest className="size-2.5" />
						{wt.pr_number && `#${wt.pr_number}`}
					</span>
				)}
				{wt.is_main && (
					<span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-primary/20 text-primary font-medium">
						main
					</span>
				)}
				{agentState && <AgentStateIcon state={agentState} />}
			</div>
			<div className="text-xs">
				{wt.dirty_count > 0 ? (
					<span className="text-warning">{wt.dirty_count} files changed</span>
				) : (
					<span className="text-success">clean</span>
				)}
			</div>
		</button>
	);
}

export function RemoteDashboard({
	worktrees,
	loading,
	onRefresh,
	onSelect,
	agentStates,
}: RemoteDashboardProps) {
	const grouped = useMemo(() => {
		const map = new Map<string, WorktreeEntryMsg[]>();
		for (const wt of worktrees) {
			const key = wt.repo_path ?? "";
			const list = map.get(key);
			if (list) {
				list.push(wt);
			} else {
				map.set(key, [wt]);
			}
		}
		return map;
	}, [worktrees]);

	const isMultiRepo = grouped.size > 1;

	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between px-3 py-2 border-b border-border">
				<span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
					Workspaces
				</span>
				<button
					type="button"
					onClick={onRefresh}
					className="p-1 hover:bg-muted rounded transition-colors"
					aria-label="Refresh"
				>
					<RefreshCw
						className={`h-3.5 w-3.5 text-muted-foreground ${loading ? "animate-spin" : ""}`}
					/>
				</button>
			</div>
			<div className="flex-1 overflow-y-auto p-3">
				{worktrees.length === 0 && !loading && (
					<p className="text-sm text-muted-foreground text-center py-8">
						No worktrees found
					</p>
				)}
				{loading && worktrees.length === 0 && (
					<p className="text-sm text-muted-foreground text-center py-8">
						Loading...
					</p>
				)}
				<div className="grid gap-3">
					{[...grouped.entries()].map(([repoPath, repoWorktrees]) => (
						<div key={repoPath || "__single"}>
							{isMultiRepo && repoPath && (
								<div className="flex items-center gap-1.5 mb-2 mt-1">
									<FolderGit2 className="size-3.5 text-muted-foreground" />
									<span className="text-xs font-medium text-muted-foreground">
										{repoDisplayName(repoPath)}
									</span>
								</div>
							)}
							<div className="grid gap-3">
								{repoWorktrees.map((wt) => (
									<WorktreeCard
										key={wt.path}
										wt={wt}
										agentState={
											agentStates
												? aggregateAgentState(agentStates, wt.path)
												: undefined
										}
										onSelect={onSelect}
									/>
								))}
							</div>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}
