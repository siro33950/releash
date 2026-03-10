import { CheckCircle2, PlayCircle, RotateCcw } from "lucide-react";
import { WorkflowTimeline } from "@/components/panels/WorkflowTimeline";
import { Button } from "@/components/ui/button";
import type { TimelineEntry } from "@/types/workflow";

interface ImplementationPanelProps {
	timelineEntries: TimelineEntry[];
	started: boolean;
	onApprovePlan?: () => void;
	onRequestRevision?: () => void;
	onApprove?: () => void;
}

export function ImplementationPanel({
	timelineEntries,
	started,
	onApprovePlan,
	onRequestRevision,
	onApprove,
}: ImplementationPanelProps) {
	return (
		<div className="flex flex-col h-full">
			<div className="flex items-center justify-end px-2 py-1 border-b border-border shrink-0">
				<div className="flex items-center gap-1">
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
				</div>
			</div>
			<div className="flex-1 overflow-hidden">
				{started ? (
					<WorkflowTimeline entries={timelineEntries} />
				) : (
					<div className="flex items-center justify-center h-full text-sm text-muted-foreground">
						Implementation has not started yet
					</div>
				)}
			</div>
		</div>
	);
}
