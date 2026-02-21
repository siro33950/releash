import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, GitPullRequest, Loader2, RefreshCw } from "lucide-react";
import { MarkdownPreview } from "@/components/panels/MarkdownPreview";
import { EmptyState } from "@/components/ui/empty-state";
import { Message } from "@/components/ui/message";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useBranchPr } from "@/hooks/useBranchPr";
import { usePrDetail } from "@/hooks/usePrDetail";
import { cn } from "@/lib/utils";

interface PullRequestPanelProps {
	rootPath: string;
	branch: string | null;
}

function StateBadge({ state }: { state: string }) {
	const color =
		state === "OPEN"
			? "bg-success/20 text-success"
			: state === "MERGED"
				? "bg-info/20 text-info"
				: "bg-destructive/20 text-destructive";
	return (
		<span
			className={cn(
				"inline-flex items-center px-2 py-0.5 rounded text-xs font-medium uppercase",
				color,
			)}
		>
			{state}
		</span>
	);
}

function ReviewStateBadge({ state }: { state: string }) {
	const label =
		state === "APPROVED"
			? "Approved"
			: state === "CHANGES_REQUESTED"
				? "Changes Requested"
				: "Commented";
	const color =
		state === "APPROVED"
			? "bg-success/20 text-success"
			: state === "CHANGES_REQUESTED"
				? "bg-warning/20 text-warning"
				: "bg-info/20 text-info";
	return (
		<span
			className={cn(
				"inline-flex items-center px-2 py-0.5 rounded text-xs font-medium",
				color,
			)}
		>
			{label}
		</span>
	);
}

function formatDate(iso: string): string {
	try {
		return new Date(iso).toLocaleDateString(undefined, {
			year: "numeric",
			month: "short",
			day: "numeric",
			hour: "2-digit",
			minute: "2-digit",
		});
	} catch {
		return iso;
	}
}

function SectionHeader({ title, count }: { title: string; count?: number }) {
	return (
		<h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground pb-1 border-b border-border">
			{title}
			{count != null && (
				<span className="ml-1.5 text-muted-foreground/60">{count}</span>
			)}
		</h3>
	);
}

export function PullRequestPanel({ rootPath, branch }: PullRequestPanelProps) {
	const { prNumber, prUrl, loading: prLoading } = useBranchPr(rootPath, branch);
	const { detail, loading, error, refresh } = usePrDetail(rootPath, prNumber);

	if (prLoading) {
		return (
			<div className="h-full flex items-center justify-center bg-sidebar">
				<Loader2 className="size-6 text-muted-foreground animate-spin" />
			</div>
		);
	}

	if (!prNumber) {
		return (
			<div className="h-full bg-sidebar">
				<EmptyState
					icon={GitPullRequest}
					title="No pull request for this branch"
				/>
			</div>
		);
	}

	if (loading && !detail) {
		return (
			<div className="h-full flex items-center justify-center bg-sidebar">
				<Loader2 className="size-6 text-muted-foreground animate-spin" />
			</div>
		);
	}

	if (error && !detail) {
		return (
			<div className="h-full flex flex-col items-center justify-center bg-sidebar">
				<Message
					variant="block"
					message="Failed to load PR details"
					onRetry={refresh}
				/>
			</div>
		);
	}

	if (!detail) return null;

	return (
		<div className="h-full flex flex-col bg-sidebar">
			{/* Header */}
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<GitPullRequest className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
				<span className="text-xs font-semibold uppercase tracking-wide truncate flex-1">
					Pull Request
				</span>
				{prUrl && (
					<button
						type="button"
						className="inline-flex items-center justify-center h-5 w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
						onClick={() => openUrl(prUrl)}
						title="Open in browser"
					>
						<ExternalLink className="h-3.5 w-3.5" />
					</button>
				)}
				<button
					type="button"
					className="inline-flex items-center justify-center h-5 w-5 rounded text-muted-foreground hover:text-foreground hover:bg-sidebar-accent-foreground/10 transition-colors shrink-0"
					onClick={refresh}
					title="Refresh"
				>
					<RefreshCw className="h-3.5 w-3.5" />
				</button>
			</div>

			<ScrollArea className="flex-1 min-h-0">
				<div className="p-4 space-y-6">
					{/* Title & meta */}
					<div className="space-y-2">
						<div className="flex items-start gap-2">
							<h2 className="text-sm font-semibold flex-1 leading-snug">
								{detail.title}
							</h2>
							<span className="text-xs text-muted-foreground shrink-0">
								#{detail.number}
							</span>
						</div>
						<div className="flex items-center gap-2 flex-wrap">
							<StateBadge state={detail.state} />
							<span className="text-xs text-muted-foreground">
								{detail.author.login}
							</span>
						</div>
						<div className="text-xs text-muted-foreground font-mono">
							{detail.base_ref_name} ← {detail.head_ref_name}
						</div>
						<div className="flex items-center gap-3 text-xs">
							<span className="text-success font-medium">
								+{detail.additions}
							</span>
							<span className="text-destructive font-medium">
								-{detail.deletions}
							</span>
							<span className="text-muted-foreground">
								{detail.changed_files} files
							</span>
						</div>
					</div>

					{/* Body */}
					{detail.body && (
						<div className="space-y-2">
							<SectionHeader title="Description" />
							<div className="border border-border rounded-md overflow-hidden">
								<MarkdownPreview content={detail.body} className="p-4" />
							</div>
						</div>
					)}

					{/* Reviews */}
					{detail.reviews.length > 0 && (
						<div className="space-y-3">
							<SectionHeader title="Reviews" count={detail.reviews.length} />
							{detail.reviews.map((review, i) => (
								<div
									key={`review-${review.author.login}-${review.submitted_at}-${i}`}
									className="border border-border rounded-md p-3 space-y-2"
								>
									<div className="flex items-center gap-2 flex-wrap">
										<ReviewStateBadge state={review.state} />
										<span className="text-xs font-medium">
											{review.author.login}
										</span>
										<span className="text-xs text-muted-foreground ml-auto">
											{formatDate(review.submitted_at)}
										</span>
									</div>
									{review.body && (
										<div className="border-t border-border pt-2">
											<MarkdownPreview content={review.body} className="p-0" />
										</div>
									)}
								</div>
							))}
						</div>
					)}

					{/* Comments */}
					{detail.comments.length > 0 && (
						<div className="space-y-3">
							<SectionHeader title="Comments" count={detail.comments.length} />
							{detail.comments.map((comment, i) => (
								<div
									key={`comment-${comment.author.login}-${comment.created_at}-${i}`}
									className="border border-border rounded-md p-3 space-y-2"
								>
									<div className="flex items-center gap-2">
										<span className="text-xs font-medium">
											{comment.author.login}
										</span>
										<span className="text-xs text-muted-foreground ml-auto">
											{formatDate(comment.created_at)}
										</span>
									</div>
									{comment.body && (
										<div className="border-t border-border pt-2">
											<MarkdownPreview content={comment.body} className="p-0" />
										</div>
									)}
								</div>
							))}
						</div>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
