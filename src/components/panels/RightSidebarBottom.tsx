import {
	Check,
	ChevronDown,
	ChevronUp,
	Copy,
	Loader2,
	MessageSquare,
	Play,
	Send,
	Terminal,
	X,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { CommentList } from "@/components/panels/CommentList";
import { ReviewModal } from "@/components/panels/ReviewModal";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useReviewExecution } from "@/hooks/useReviewExecution";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import type { LineComment } from "@/types/comment";
import type { AppSettings, Theme } from "@/types/settings";

type RightBottomTab = "terminal" | "review";

interface RightSidebarBottomProps {
	rootPath: string;
	theme?: Theme;
	settings: AppSettings;
	comments: LineComment[];
	onCommentClick?: (filePath: string, lineNumber: number) => void;
	onDeleteComment?: (id: string) => void;
	onResolveComment?: (id: string) => void;
	onSendToTerminal?: (comments: LineComment[]) => void;
	showResolvedComments?: boolean;
	onToggleShowResolved?: () => void;
	onToggleCollapse?: () => void;
	collapsed?: boolean;
}

export function RightSidebarBottom({
	rootPath,
	theme,
	settings,
	comments,
	onCommentClick,
	onDeleteComment,
	onResolveComment,
	onSendToTerminal,
	showResolvedComments,
	onToggleShowResolved,
	onToggleCollapse,
	collapsed,
}: RightSidebarBottomProps) {
	const [activeTab, setActiveTab] = useState<RightBottomTab>("terminal");
	const [reviewModalOpen, setReviewModalOpen] = useState(false);
	const unsentComments = comments.filter((c) => c.status === "unsent");

	const {
		status,
		summary,
		progress,
		fileStates,
		startReview,
		cancelReview,
		reset,
	} = useReviewExecution(rootPath, comments, settings);

	const isRunning = status === "starting" || status === "running";
	const isFinished =
		status === "completed" || status === "error" || status === "cancelled";
	const reviewDisabled = settings.reviewAgent === "none";

	const handleStartReview = () => {
		if (isFinished) reset();
		startReview();
	};

	const reviewDot = isRunning
		? "bg-blue-400 animate-pulse"
		: status === "completed"
			? "bg-green-400"
			: status === "error"
				? "bg-destructive"
				: null;

	const unresolvedComments = useMemo(
		() => comments.filter((c) => !c.resolved),
		[comments],
	);

	const handleCopyComments = useCallback(() => {
		const text = formatCommentsForTerminal(unresolvedComments);
		navigator.clipboard.writeText(text);
	}, [unresolvedComments]);

	return (
		<div className="flex flex-col h-full">
			<Tabs
				value={activeTab}
				onValueChange={(val) => setActiveTab(val as RightBottomTab)}
				className="flex flex-col h-full"
			>
				<div className="flex items-center gap-2 shrink-0 px-0 pt-0 bg-background">
					{onToggleCollapse && (
						<button
							type="button"
							onClick={onToggleCollapse}
							className="shrink-0 p-1 ml-2 text-muted-foreground hover:text-foreground transition-colors"
							aria-label={collapsed ? "Expand panel" : "Collapse panel"}
						>
							{collapsed ? (
								<ChevronUp className="size-3.5" />
							) : (
								<ChevronDown className="size-3.5" />
							)}
						</button>
					)}
					<TabsList variant="line" aria-label="Bottom sidebar tabs">
						<TabsTrigger value="terminal" aria-label="Terminal">
							<span className="inline-flex items-center">
								<Terminal className="size-3.5" />
							</span>
						</TabsTrigger>
						<TabsTrigger value="review" aria-label="Review">
							<span className="inline-flex items-center gap-1.5">
								<MessageSquare className="size-3.5" />
								{reviewDot && (
									<span className={`size-1.5 rounded-full ${reviewDot}`} />
								)}
								{!reviewDot && unresolvedComments.length > 0 && (
									<span className="px-1 text-[10px] bg-primary/20 text-primary rounded">
										{unresolvedComments.length}
									</span>
								)}
							</span>
						</TabsTrigger>
					</TabsList>
				</div>
				<TabsContent
					value="terminal"
					forceMount
					className="flex-1 overflow-hidden data-[state=inactive]:hidden"
				>
					<TerminalPanel cwd={rootPath} theme={theme} />
				</TabsContent>
				<TabsContent
					value="review"
					className="flex-1 overflow-hidden flex flex-col"
				>
					<div className="flex-1 min-h-0 overflow-hidden">
						<CommentList
							comments={comments}
							onCommentClick={onCommentClick}
							onDeleteComment={onDeleteComment}
							onResolveComment={onResolveComment}
							showResolvedComments={showResolvedComments}
							onToggleShowResolved={onToggleShowResolved}
						/>
					</div>
					<div className="shrink-0 px-3 py-2 border-t border-border flex items-center gap-2">
						{!reviewDisabled && (
							<Button
								variant="ghost"
								size="xs"
								className="flex-1"
								onClick={
									status === "idle"
										? handleStartReview
										: () => setReviewModalOpen(true)
								}
								disabled={false}
							>
								{status === "idle" && <Play className="size-3" />}
								{isRunning && <Loader2 className="size-3 animate-spin" />}
								{status === "completed" && (
									<Check className="size-3 text-green-400" />
								)}
								{(status === "error" || status === "cancelled") && (
									<X className="size-3 text-destructive" />
								)}
								AI Review
							</Button>
						)}
						<Button
							variant="ghost"
							size="icon-xs"
							onClick={handleCopyComments}
							disabled={unresolvedComments.length === 0}
							aria-label="Copy comments to clipboard"
						>
							<Copy />
						</Button>
						{onSendToTerminal && (
							<Button
								variant="ghost"
								size="xs"
								onClick={() => onSendToTerminal(unsentComments)}
								disabled={unsentComments.length === 0}
							>
								<Send />
								Send
							</Button>
						)}
					</div>
				</TabsContent>
			</Tabs>
			<ReviewModal
				open={reviewModalOpen}
				onOpenChange={setReviewModalOpen}
				status={status}
				summary={summary}
				progress={progress}
				fileStates={fileStates}
				onCancel={cancelReview}
				onRetry={handleStartReview}
			/>
		</div>
	);
}
