import { GitBranch, Lock, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { WorktreeEntry } from "@/types/git";

interface WorktreeCardProps {
	worktree: WorktreeEntry;
	repoPath: string;
	onOpen: (path: string) => void;
	onDelete: (worktree: WorktreeEntry) => void;
}

function relativePath(worktreePath: string, repoPath: string): string {
	const repoParent = repoPath.replace(/\/[^/]+\/?$/, "");
	if (worktreePath.startsWith(repoParent)) {
		return `./${worktreePath.slice(repoParent.length + 1)}`;
	}
	return worktreePath;
}

export function WorktreeCard({
	worktree,
	repoPath,
	onOpen,
	onDelete,
}: WorktreeCardProps) {
	return (
		<div className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4 hover:border-primary/50 transition-colors">
			<div className="flex items-center gap-2 min-w-0">
				<GitBranch className="size-4 shrink-0 text-muted-foreground" />
				<span className="text-sm font-medium truncate">{worktree.branch}</span>
				{worktree.is_locked && (
					<Lock className="size-3.5 shrink-0 text-yellow-500" />
				)}
			</div>

			<div className="text-xs text-muted-foreground truncate">
				{relativePath(worktree.path, repoPath)}
			</div>

			<div className="text-xs">
				{worktree.dirty_count > 0 ? (
					<span className="text-yellow-500">
						{worktree.dirty_count} files changed
					</span>
				) : (
					<span className="text-green-500">clean</span>
				)}
			</div>

			<div className="flex items-center gap-2 mt-auto">
				<Button
					size="sm"
					className="flex-1"
					onClick={() => onOpen(worktree.path)}
				>
					Open
				</Button>
				{!worktree.is_main && (
					<Button
						size="icon-sm"
						variant="ghost"
						onClick={() => onDelete(worktree)}
						aria-label={`Delete ${worktree.name}`}
					>
						<Trash2 className="size-4 text-muted-foreground" />
					</Button>
				)}
			</div>
		</div>
	);
}
