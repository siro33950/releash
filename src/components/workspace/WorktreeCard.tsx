import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, GitBranch, Loader2, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { BranchCard as BranchCardType } from "@/types/git";

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
				{hasWorktree && (
					<Button
						size="icon-sm"
						variant="ghost"
						onClick={() => onDelete(branch)}
						aria-label={`Delete worktree for ${branch.name}`}
					>
						<Trash2 className="size-4 text-muted-foreground" />
					</Button>
				)}
			</div>
		</div>
	);
}
