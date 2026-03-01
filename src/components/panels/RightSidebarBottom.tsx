import {
	Check,
	ChevronDown,
	ChevronUp,
	Loader2,
	MessageSquare,
	Play,
	Send,
	Terminal,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { CommentList } from "@/components/panels/CommentList";
import { ReviewModal } from "@/components/panels/ReviewModal";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useReviewExecution } from "@/hooks/useReviewExecution";
import { useSkills } from "@/hooks/useSkills";
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

	const { skills } = useSkills(rootPath);
	const { status, summary, output, startReview, cancelReview, reset } =
		useReviewExecution(rootPath, comments, settings);

	const selectedSkill = useMemo(
		() =>
			skills.find((s) => s.name === settings.defaultReviewSkill) ?? skills[0],
		[skills, settings.defaultReviewSkill],
	);

	const isRunning = status === "starting" || status === "running";
	const isFinished =
		status === "completed" || status === "error" || status === "cancelled";
	const reviewDisabled = settings.reviewAgent === "none";

	const handleStartReview = () => {
		if (isFinished) reset();
		if (selectedSkill) startReview(selectedSkill);
	};

	const reviewDot = isRunning
		? "bg-blue-400 animate-pulse"
		: status === "completed"
			? "bg-green-400"
			: status === "error"
				? "bg-destructive"
				: null;

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
								{!reviewDot && unsentComments.length > 0 && (
									<span className="px-1 text-[10px] bg-primary/20 text-primary rounded">
										{unsentComments.length}
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
							<button
								type="button"
								onClick={
									status === "idle"
										? handleStartReview
										: () => setReviewModalOpen(true)
								}
								disabled={
									status === "idle" &&
									(!selectedSkill || skills.length === 0)
								}
								className="flex-1 flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md bg-primary/10 text-primary hover:bg-primary/20 transition-colors disabled:opacity-50 disabled:pointer-events-none"
							>
								{status === "idle" && (
									<Play className="h-3.5 w-3.5" />
								)}
								{isRunning && (
									<Loader2 className="h-3.5 w-3.5 animate-spin" />
								)}
								{status === "completed" && (
									<Check className="h-3.5 w-3.5 text-green-400" />
								)}
								{(status === "error" || status === "cancelled") && (
									<X className="h-3.5 w-3.5 text-destructive" />
								)}
								AI Review
							</button>
						)}
						{onSendToTerminal && (
							<button
								type="button"
								onClick={() => onSendToTerminal(unsentComments)}
								disabled={unsentComments.length === 0}
								className="flex items-center justify-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md bg-primary/10 text-primary hover:bg-primary/20 transition-colors disabled:opacity-50 disabled:pointer-events-none"
								title="Send unsent comments to terminal"
							>
								<Send className="h-3.5 w-3.5" />
								Send
							</button>
						)}
					</div>
				</TabsContent>
			</Tabs>
			<ReviewModal
				open={reviewModalOpen}
				onOpenChange={setReviewModalOpen}
				status={status}
				summary={summary}
				output={output}
				onCancel={cancelReview}
				onRetry={handleStartReview}
			/>
		</div>
	);
}
