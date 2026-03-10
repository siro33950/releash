import { CheckCircle2, RotateCcw } from "lucide-react";
import { WorkflowTimeline } from "@/components/panels/WorkflowTimeline";
import { Button } from "@/components/ui/button";
import type { TimelineEntry } from "@/types/workflow";

interface PlanPanelProps {
	timelineEntries: TimelineEntry[];
	onRequirementsComplete?: () => void;
	onRequestRevision?: () => void;
}

export function PlanPanel({
	timelineEntries,
	onRequirementsComplete,
	onRequestRevision,
}: PlanPanelProps) {
	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-end px-2 py-1 border-b border-border shrink-0">
				<div className="flex items-center gap-1">
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
				</div>
			</div>
			<div className="flex-1 overflow-hidden">
				<WorkflowTimeline entries={timelineEntries} />
			</div>
		</div>
	);
}
