import { GitBranch, Lock, RefreshCw } from "lucide-react";
import type { WorktreeEntryMsg } from "@/types/protocol";

interface RemoteDashboardProps {
	worktrees: WorktreeEntryMsg[];
	loading: boolean;
	onRefresh: () => void;
	onSelect?: (worktreePath: string) => void;
}

export function RemoteDashboard({
	worktrees,
	loading,
	onRefresh,
	onSelect,
}: RemoteDashboardProps) {
	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-between px-3 py-2 border-b border-neutral-800">
				<span className="text-xs font-semibold text-neutral-400 uppercase tracking-wider">
					Workspaces
				</span>
				<button
					type="button"
					onClick={onRefresh}
					className="p-1 hover:bg-neutral-800 rounded transition-colors"
					aria-label="Refresh"
				>
					<RefreshCw
						className={`h-3.5 w-3.5 text-neutral-400 ${loading ? "animate-spin" : ""}`}
					/>
				</button>
			</div>
			<div className="flex-1 overflow-y-auto p-3">
				{worktrees.length === 0 && !loading && (
					<p className="text-sm text-neutral-500 text-center py-8">
						No worktrees found
					</p>
				)}
				{loading && worktrees.length === 0 && (
					<p className="text-sm text-neutral-500 text-center py-8">
						Loading...
					</p>
				)}
				<div className="grid gap-3">
					{worktrees.map((wt) => (
						<button
							key={wt.path}
							type="button"
							className="flex items-start gap-3 p-3 rounded-lg border border-neutral-800 bg-neutral-900/50 hover:border-neutral-600 hover:bg-neutral-800/50 transition-colors text-left w-full"
							onClick={() => onSelect?.(wt.path)}
						>
							<GitBranch className="size-4 shrink-0 text-neutral-500 mt-0.5" />
							<div className="flex-1 min-w-0">
								<div className="flex items-center gap-1.5">
									<span className="text-sm font-medium truncate">
										{wt.branch}
									</span>
									{wt.is_locked && (
										<Lock className="size-3 text-yellow-500 shrink-0" />
									)}
									{wt.is_main && (
										<span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/20 text-blue-400">
											main
										</span>
									)}
								</div>
								<div className="text-xs text-neutral-500 truncate mt-0.5">
									{wt.name}
								</div>
								<div className="text-xs mt-1">
									{wt.dirty_count > 0 ? (
										<span className="text-yellow-500">
											{wt.dirty_count} changes
										</span>
									) : (
										<span className="text-green-500">clean</span>
									)}
								</div>
							</div>
						</button>
					))}
				</div>
			</div>
		</div>
	);
}
