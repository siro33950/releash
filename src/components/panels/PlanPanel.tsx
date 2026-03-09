import { CheckCircle2, RotateCcw } from "lucide-react";
import { WorkflowPanel } from "@/components/panels/WorkflowPanel";
import { WorkflowTimeline } from "@/components/panels/WorkflowTimeline";
import { Button } from "@/components/ui/button";
import type { Thread } from "@/types/thread";
import type { TimelineEntry } from "@/types/workflow";

interface PlanPanelProps {
	timelineEntries: TimelineEntry[];
	threads: Thread[];
	onThreadClick?: (filePath: string, lineNumber: number) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onRequirementsComplete?: () => void;
	onRequestRevision?: () => void;
}

export function PlanPanel({
	timelineEntries,
	threads,
	onThreadClick,
	onDeleteThread,
	onResolveThread,
	onRequirementsComplete,
	onRequestRevision,
}: PlanPanelProps) {
	return (
		<WorkflowPanel
			timelineContent={<WorkflowTimeline entries={timelineEntries} />}
			actions={
				<>
					{onRequirementsComplete && (
						<Button
							variant="ghost"
							size="sm"
							className="h-6 text-xs gap-1"
							onClick={onRequirementsComplete}
						>
							<CheckCircle2 className="h-3 w-3" />
							Complete
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
				</>
			}
			threads={threads}
			onThreadClick={onThreadClick}
			onDeleteThread={onDeleteThread}
			onResolveThread={onResolveThread}
		/>
	);
}
