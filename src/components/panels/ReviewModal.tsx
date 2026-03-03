import {
	AlertTriangle,
	CheckCircle2,
	Circle,
	Info,
	Lightbulb,
	Loader2,
	Play,
	Square,
	XCircle,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import type {
	FileReviewState,
	ReviewStatus,
	ReviewSummary,
} from "@/hooks/useReviewExecution";
import { parseStreamJson } from "@/lib/parseStreamJson";

export function StatusIndicator({
	status,
	progress,
}: {
	status: ReviewStatus;
	progress?: { done: number; total: number } | null;
}) {
	switch (status) {
		case "starting":
		case "running":
			return (
				<span className="flex items-center gap-1.5 text-xs text-blue-400">
					<Loader2 className="h-3.5 w-3.5 animate-spin" />
					{progress
						? `Reviewing (${progress.done}/${progress.total})`
						: status === "starting"
							? "Starting..."
							: "Reviewing..."}
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

export function SummaryDisplay({ summary }: { summary: ReviewSummary }) {
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

function FileStatusIcon({ status }: { status: FileReviewState["status"] }) {
	switch (status) {
		case "pending":
			return <Circle className="h-3.5 w-3.5 text-muted-foreground" />;
		case "running":
			return <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-400" />;
		case "done":
			return <CheckCircle2 className="h-3.5 w-3.5 text-status-added" />;
		case "error":
			return <XCircle className="h-3.5 w-3.5 text-destructive" />;
	}
}

interface ReviewModalProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	status: ReviewStatus;
	summary: ReviewSummary | null;
	progress: { done: number; total: number } | null;
	fileStates: FileReviewState[];
	onCancel: () => void;
	onRetry: () => void;
}

export function ReviewModal({
	open,
	onOpenChange,
	status,
	summary,
	progress,
	fileStates,
	onCancel,
	onRetry,
}: ReviewModalProps) {
	const isRunning = status === "starting" || status === "running";
	const hasFiles = fileStates.length > 0;

	const [selectedFile, setSelectedFile] = useState<string | null>(null);

	// Auto-select first running file, or first file
	useEffect(() => {
		if (!hasFiles) {
			setSelectedFile(null);
			return;
		}
		// If current selection is still valid, keep it
		if (selectedFile && fileStates.some((f) => f.filePath === selectedFile)) {
			return;
		}
		const running = fileStates.find((f) => f.status === "running");
		setSelectedFile(running?.filePath ?? fileStates[0]?.filePath ?? null);
	}, [hasFiles, fileStates, selectedFile]);

	const selectedState = useMemo(
		() => fileStates.find((f) => f.filePath === selectedFile) ?? null,
		[fileStates, selectedFile],
	);

	const parsedOutput = useMemo(
		() =>
			selectedState
				? parseStreamJson(selectedState.output).replace(/^\n+/, "")
				: "",
		[selectedState],
	);

	const outputRef = useRef<HTMLDivElement>(null);

	// Auto-scroll to bottom when new output arrives for selected file
	// biome-ignore lint/correctness/useExhaustiveDependencies: trigger scroll on output change
	useEffect(() => {
		const el = outputRef.current;
		if (el) {
			el.scrollTop = el.scrollHeight;
		}
	}, [parsedOutput]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="w-[80vw] max-w-[1100px] h-[70vh] flex flex-col gap-0 p-0">
				<DialogHeader className="px-6 py-4 shrink-0 border-b border-border">
					<div className="flex items-center gap-3">
						<DialogTitle>AI Review Log</DialogTitle>
						<StatusIndicator status={status} progress={progress} />
					</div>
					<DialogDescription className="sr-only">
						AI code review log output per file
					</DialogDescription>
				</DialogHeader>

				<div className="flex-1 min-h-0 flex">
					{/* Left pane: file list */}
					<div className="w-56 shrink-0 border-r border-border overflow-y-auto">
						{fileStates.map((file) => {
							const fileName = file.filePath.split("/").pop() ?? file.filePath;
							const isSelected = file.filePath === selectedFile;
							return (
								<button
									key={file.filePath}
									type="button"
									onClick={() => setSelectedFile(file.filePath)}
									className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent/50 transition-colors ${
										isSelected ? "bg-accent" : ""
									}`}
									title={file.filePath}
								>
									<FileStatusIcon status={file.status} />
									<span className="truncate">{fileName}</span>
								</button>
							);
						})}
						{!hasFiles && (
							<div className="p-3 text-xs text-muted-foreground">
								{status === "idle" ? "No review running" : "Loading files..."}
							</div>
						)}
					</div>

					{/* Right pane: selected file output */}
					<div
						ref={outputRef}
						className="flex-1 min-h-0 overflow-auto p-3 text-sm text-foreground whitespace-pre-wrap break-words select-text"
					>
						{parsedOutput || (
							<span className="text-muted-foreground">
								{!selectedState
									? "Select a file to view output"
									: selectedState.status === "pending"
										? "Waiting to start..."
										: selectedState.status === "running"
											? "Waiting for output..."
											: "No output"}
							</span>
						)}
					</div>
				</div>

				<DialogFooter className="px-6 py-4 shrink-0 border-t border-border flex items-center">
					<div className="flex-1">
						{summary && <SummaryDisplay summary={summary} />}
					</div>
					{isRunning && (
						<Button variant="destructive" size="sm" onClick={onCancel}>
							<Square className="h-3.5 w-3.5 mr-1.5" />
							Cancel
						</Button>
					)}
					{!isRunning && status !== "idle" && (
						<Button variant="outline" size="sm" onClick={onRetry}>
							<Play className="h-3.5 w-3.5 mr-1.5" />
							Retry
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
