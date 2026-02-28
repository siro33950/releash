import {
	AlertTriangle,
	Info,
	Lightbulb,
	Loader2,
	Play,
	Square,
	XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import {
	type ReviewStatus,
	type ReviewSummary,
	useReviewExecution,
} from "@/hooks/useReviewExecution";
import { useSkills } from "@/hooks/useSkills";
import { parseStreamJson } from "@/lib/parseStreamJson";
import type { LineComment } from "@/types/comment";
import type { AppSettings } from "@/types/settings";

function StatusIndicator({ status }: { status: ReviewStatus }) {
	switch (status) {
		case "starting":
		case "running":
			return (
				<span className="flex items-center gap-1.5 text-xs text-blue-400">
					<Loader2 className="h-3.5 w-3.5 animate-spin" />
					{status === "starting" ? "Starting..." : "Reviewing..."}
				</span>
			);
		case "completed":
			return <span className="text-xs text-status-added">Completed</span>;
		case "error":
			return <span className="text-xs text-destructive">Error</span>;
		case "cancelled":
			return <span className="text-xs text-muted-foreground">Cancelled</span>;
		default:
			return null;
	}
}

function SummaryDisplay({ summary }: { summary: ReviewSummary }) {
	if (summary.total === 0) {
		return (
			<span className="text-xs text-muted-foreground">No issues found</span>
		);
	}

	return (
		<div className="flex items-center gap-2 text-xs">
			<span className="text-muted-foreground">{summary.total} issues</span>
			{summary.errors > 0 && (
				<span className="flex items-center gap-0.5 text-destructive">
					<XCircle className="h-3.5 w-3.5" />
					{summary.errors}
				</span>
			)}
			{summary.warnings > 0 && (
				<span className="flex items-center gap-0.5 text-yellow-500">
					<AlertTriangle className="h-3.5 w-3.5" />
					{summary.warnings}
				</span>
			)}
			{summary.infos > 0 && (
				<span className="flex items-center gap-0.5 text-blue-400">
					<Info className="h-3.5 w-3.5" />
					{summary.infos}
				</span>
			)}
			{summary.suggestions > 0 && (
				<span className="flex items-center gap-0.5 text-green-400">
					<Lightbulb className="h-3.5 w-3.5" />
					{summary.suggestions}
				</span>
			)}
		</div>
	);
}

interface ReviewModalProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	rootPath: string | null;
	comments: LineComment[];
	settings: AppSettings;
}

export function ReviewModal({
	open,
	onOpenChange,
	rootPath,
	comments,
	settings,
}: ReviewModalProps) {
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

	const outputRef = useRef<HTMLPreElement>(null);
	const parsedOutput = useMemo(
		() => parseStreamJson(output).replace(/^\n+/, ""),
		[output],
	);

	// Auto-scroll to bottom when new output arrives
	// biome-ignore lint/correctness/useExhaustiveDependencies: trigger scroll on output change
	useEffect(() => {
		const el = outputRef.current;
		if (el) {
			el.scrollTop = el.scrollHeight;
		}
	}, [parsedOutput]);

	// Reset state when modal closes
	const handleOpenChange = (nextOpen: boolean) => {
		if (!nextOpen && isFinished) {
			reset();
		}
		onOpenChange(nextOpen);
	};

	const handleStart = () => {
		if (isFinished) reset();
		if (selectedSkill) startReview(selectedSkill);
	};

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="w-[70vw] max-w-[900px] h-[60vh] flex flex-col gap-0 p-0">
				<DialogHeader className="px-6 py-4 shrink-0 border-b border-border">
					<div className="flex items-center gap-3">
						<DialogTitle>AI Review</DialogTitle>
						<StatusIndicator status={status} />
					</div>
					<DialogDescription className="sr-only">
						AI code review progress
					</DialogDescription>
				</DialogHeader>

				<div
					ref={outputRef}
					className="flex-1 min-h-0 overflow-auto p-3 text-sm text-foreground whitespace-pre-wrap break-words"
				>
					{parsedOutput || (
						<span className="text-muted-foreground">
							{reviewDisabled
								? "Review agent is not configured. Go to Settings > Review to select an agent."
								: status === "idle"
									? "Press Start to begin AI review"
									: "Waiting for output..."}
						</span>
					)}
				</div>

				<DialogFooter className="px-6 py-4 shrink-0 border-t border-border flex items-center">
					<div className="flex-1">
						{summary && <SummaryDisplay summary={summary} />}
					</div>
					{isRunning ? (
						<Button variant="destructive" size="sm" onClick={cancelReview}>
							<Square className="h-3.5 w-3.5 mr-1.5" />
							Cancel
						</Button>
					) : (
						<Button
							size="sm"
							onClick={handleStart}
							disabled={reviewDisabled || !selectedSkill || skills.length === 0}
						>
							<Play className="h-3.5 w-3.5 mr-1.5" />
							{isFinished ? "Retry" : "Start"}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
