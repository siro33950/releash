import { CheckCircle2, Loader2, Square, XCircle } from "lucide-react";
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
import type { ThreadAITask, ThreadAITaskStatus } from "@/hooks/useThreadAI";
import { parseStreamJson } from "@/lib/parseStreamJson";

function TaskStatusIcon({ status }: { status: ThreadAITaskStatus }) {
	switch (status) {
		case "running":
			return <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-400" />;
		case "completed":
			return <CheckCircle2 className="h-3.5 w-3.5 text-status-added" />;
		case "error":
			return <XCircle className="h-3.5 w-3.5 text-destructive" />;
	}
}

interface ThreadAIModalProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	tasks: Map<string, ThreadAITask>;
	onCancelTask: (threadId: string) => void;
	initialThreadId?: string | null;
}

export function ThreadAIModal({
	open,
	onOpenChange,
	tasks,
	onCancelTask,
	initialThreadId,
}: ThreadAIModalProps) {
	const taskList = useMemo(() => Array.from(tasks.values()), [tasks]);
	const hasTasks = taskList.length > 0;

	const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);

	// When opened with a specific thread, select it (only once per open);
	// otherwise auto-select first running task or first task
	const appliedRef = useRef(false);

	useEffect(() => {
		if (!open) {
			appliedRef.current = false;
			return;
		}
		// Prioritize initialThreadId (once per open)
		if (
			!appliedRef.current &&
			initialThreadId &&
			taskList.some((t) => t.threadId === initialThreadId)
		) {
			appliedRef.current = true;
			setSelectedThreadId(initialThreadId);
			return;
		}
		// Auto-select fallback
		if (!hasTasks) {
			setSelectedThreadId(null);
			return;
		}
		if (
			selectedThreadId &&
			taskList.some((t) => t.threadId === selectedThreadId)
		) {
			return;
		}
		const running = taskList.find((t) => t.status === "running");
		setSelectedThreadId(running?.threadId ?? taskList[0]?.threadId ?? null);
	}, [open, initialThreadId, hasTasks, taskList, selectedThreadId]);

	const selectedTask = useMemo(
		() => taskList.find((t) => t.threadId === selectedThreadId) ?? null,
		[taskList, selectedThreadId],
	);

	const parsedOutput = useMemo(
		() =>
			selectedTask
				? parseStreamJson(selectedTask.output).replace(/^\n+/, "")
				: "",
		[selectedTask],
	);

	const outputRef = useRef<HTMLDivElement>(null);

	// Auto-scroll to bottom when new output arrives (only if near bottom)
	// biome-ignore lint/correctness/useExhaustiveDependencies: trigger scroll on output change
	useEffect(() => {
		const el = outputRef.current;
		if (el) {
			const isNearBottom =
				el.scrollHeight - el.scrollTop - el.clientHeight < 40;
			if (isNearBottom) {
				el.scrollTop = el.scrollHeight;
			}
		}
	}, [parsedOutput]);

	const runningCount = useMemo(
		() => taskList.filter((t) => t.status === "running").length,
		[taskList],
	);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="w-[80vw] max-w-[1100px] h-[70vh] flex flex-col gap-0 p-0">
				<DialogHeader className="px-6 py-4 shrink-0 border-b border-border">
					<div className="flex items-center gap-3">
						<DialogTitle>Thread AI Log</DialogTitle>
						{runningCount > 0 && (
							<span className="flex items-center gap-1.5 text-xs text-blue-400">
								<Loader2 className="h-3.5 w-3.5 animate-spin" />
								{runningCount} running
							</span>
						)}
					</div>
					<DialogDescription className="sr-only">
						Thread AI processing log output
					</DialogDescription>
				</DialogHeader>

				<div className="flex-1 min-h-0 flex">
					{/* Left pane: task list */}
					<div className="w-56 shrink-0 border-r border-border overflow-y-auto">
						{taskList.map((task) => {
							const fileName = task.filePath.split("/").pop() ?? task.filePath;
							const label = fileName
								? `${fileName}${task.lineInfo ? `:${task.lineInfo}` : ""}`
								: task.threadId.slice(0, 8);
							const isSelected = task.threadId === selectedThreadId;
							return (
								<button
									key={task.threadId}
									type="button"
									onClick={() => setSelectedThreadId(task.threadId)}
									className={`w-full flex items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-accent/50 transition-colors ${
										isSelected ? "bg-accent" : ""
									}`}
									title={`${task.filePath} (${task.mode})`}
								>
									<TaskStatusIcon status={task.status} />
									<span className="truncate">{label}</span>
								</button>
							);
						})}
						{!hasTasks && (
							<div className="p-3 text-xs text-muted-foreground">
								No AI tasks
							</div>
						)}
					</div>

					{/* Right pane: selected task output */}
					<div
						ref={outputRef}
						className="flex-1 min-h-0 overflow-auto p-3 text-sm text-foreground whitespace-pre-wrap break-words select-text"
					>
						{parsedOutput || (
							<span className="text-muted-foreground">
								{!selectedTask
									? "Select a task to view output"
									: selectedTask.status === "running"
										? "Waiting for output..."
										: "No output"}
							</span>
						)}
					</div>
				</div>

				<DialogFooter className="px-6 py-4 shrink-0 border-t border-border flex items-center">
					<div className="flex-1" />
					{selectedTask?.status === "running" && (
						<Button
							variant="destructive"
							size="sm"
							onClick={() => onCancelTask(selectedTask.threadId)}
						>
							<Square className="h-3.5 w-3.5 mr-1.5" />
							Cancel
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
