import { openUrl } from "@tauri-apps/plugin-opener";
import {
	ExternalLink,
	GitBranch,
	Globe,
	Loader2,
	Monitor,
	Trash2,
} from "lucide-react";
import { AgentStateBadge } from "@/components/ui/agent-state-badge";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
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
	const hasAheadBehind = branch.ahead > 0 || branch.behind > 0;
	const isLocalOnly = !branch.is_remote_only && !branch.has_upstream;
	const hasStatusBadges =
		branch.is_default ||
		branch.agent_state ||
		(hasWorktree && branch.dirty_count > 0);

	const BranchIcon = branch.is_remote_only
		? Globe
		: isLocalOnly
			? Monitor
			: GitBranch;

	const hasSecondRow =
		hasAheadBehind ||
		branch.is_merged ||
		(branch.has_pr && branch.pr_url && branch.pr_number != null) ||
		hasStatusBadges;

	return (
		// biome-ignore lint/a11y/useSemanticElements: <button> cannot nest <button> (PR badge, delete btn)
		<div
			role="button"
			tabIndex={0}
			data-testid={`branch-card-${branch.name}`}
			className={`group relative flex flex-col gap-3 rounded-lg border p-3 shadow-sm transition-[color,border-color,box-shadow] text-left outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 ${opening ? "cursor-default" : "cursor-pointer"} ${
				hasWorktree
					? "border-border bg-card hover:border-primary/50 hover:shadow-md"
					: "border-border/50 bg-card/50 hover:border-border hover:shadow-md"
			}`}
			onClick={opening ? undefined : onOpen}
			onKeyDown={(e) => {
				if (e.target !== e.currentTarget) return;
				if ((e.key === "Enter" || e.key === " ") && !opening) {
					e.preventDefault();
					onOpen();
				}
			}}
		>
			{/* 1行目: ブランチアイコン + ブランチ名 */}
			<div className="flex items-center gap-2 min-w-0">
				<BranchIcon
					className={`size-4 shrink-0 ${hasWorktree ? "text-muted-foreground" : "text-muted-foreground/50"}`}
				/>
				<Tooltip>
					<TooltipTrigger asChild>
						<span
							className={`text-sm font-medium truncate ${!hasWorktree ? "text-muted-foreground" : ""}`}
						>
							{branch.name}
						</span>
					</TooltipTrigger>
					<TooltipContent>{branch.name}</TooltipContent>
				</Tooltip>
				{opening && (
					<Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
				)}
			</div>

			{/* 2行目: バッジ類（インデントなし） */}
			{hasSecondRow && (
				<div className="flex items-center gap-1.5 flex-wrap">
					{hasAheadBehind && (
						<span className="shrink-0 text-[10px] text-muted-foreground">
							{branch.ahead > 0 && `↑${branch.ahead}`}
							{branch.ahead > 0 && branch.behind > 0 && " "}
							{branch.behind > 0 && `↓${branch.behind}`}
						</span>
					)}
					{branch.is_merged && (
						<span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-muted text-muted-foreground font-medium">
							merged
						</span>
					)}
					{branch.has_pr && branch.pr_url && branch.pr_number != null && (
						<button
							type="button"
							className="shrink-0 inline-flex items-center gap-0.5 text-[10px] px-1.5 py-0.5 rounded bg-info/15 text-info font-medium hover:bg-info/25 transition-colors"
							onClick={(e) => {
								e.stopPropagation();
								openUrl(branch.pr_url as string);
							}}
						>
							#{branch.pr_number}
							<ExternalLink className="size-2.5" />
						</button>
					)}
					{branch.is_default && (
						<span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success font-medium">
							base
						</span>
					)}
					{branch.agent_state && (
						<AgentStateBadge
							state={branch.agent_state}
							timestamp={branch.agent_state_timestamp}
						/>
					)}
					{hasWorktree && branch.dirty_count > 0 && (
						<span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-medium">
							{branch.dirty_count} changed
						</span>
					)}
				</div>
			)}

			{/* 削除ボタン: hover時のみ表示 */}
			{!branch.is_default && (hasWorktree || branch.is_merged) && (
				<Button
					size="icon-xs"
					variant="ghost"
					className="absolute top-2 right-2 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
					onClick={(e) => {
						e.stopPropagation();
						onDelete(branch);
					}}
					aria-label={
						hasWorktree
							? `Delete worktree for ${branch.name}`
							: `Delete branch ${branch.name}`
					}
				>
					<Trash2 className="size-3 text-muted-foreground" />
				</Button>
			)}
		</div>
	);
}
