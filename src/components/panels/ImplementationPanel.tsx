import { CheckCircle2, PlayCircle, RotateCcw } from "lucide-react";
import { WorkflowPanel } from "@/components/panels/WorkflowPanel";
import { WorkflowTimeline } from "@/components/panels/WorkflowTimeline";
import { Button } from "@/components/ui/button";
import type { Thread } from "@/types/thread";
import type { TimelineEntry } from "@/types/workflow";

interface ImplementationPanelProps {
	timelineEntries: TimelineEntry[];
	threads: Thread[];
	started: boolean;
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onApprovePlan?: () => void;
	onRequestRevision?: () => void;
	onApprove?: () => void;
}

export function ImplementationPanel({
	timelineEntries,
	threads,
	started,
	onThreadClick,
	onDeleteThread,
	onResolveThread,
	onApprovePlan,
	onRequestRevision,
	onApprove,
}: ImplementationPanelProps) {
	return (
		<WorkflowPanel
			timelineContent={
				started ? (
					<WorkflowTimeline entries={timelineEntries} />
				) : (
					<div className="flex items-center justify-center h-full text-sm text-muted-foreground">
						Implementation has not started yet
					</div>
				)
			}
			actions={
				<>
					{onApprovePlan && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 text-xs gap-1"
							onClick={onApprovePlan}
						>
							<PlayCircle className="h-3 w-3" />
							Approve Plan
						</Button>
					)}
					{onRequestRevision && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 text-xs gap-1"
							onClick={onRequestRevision}
						>
							<RotateCcw className="h-3 w-3" />
							Revise
						</Button>
					)}
					{onApprove && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 text-xs gap-1"
							onClick={onApprove}
						>
							<CheckCircle2 className="h-3 w-3" />
							Approve
						</Button>
					)}
				</>
			}
			threads={threads}
			onThreadClick={onThreadClick}
			onDeleteThread={onDeleteThread}
			onResolveThread={onResolveThread}
		/>
	);
}
