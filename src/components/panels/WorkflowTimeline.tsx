import { CheckCircle2, Circle, Loader2, XCircle } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { TimelineEntry, TimelineEntryStatus } from "@/types/workflow";

function StatusIcon({ status }: { status: TimelineEntryStatus }) {
	switch (status) {
		case "completed":
			return <CheckCircle2 className="h-4 w-4 shrink-0 text-green-500" />;
		case "in_progress":
			return (
				<Loader2 className="h-4 w-4 shrink-0 text-blue-500 animate-spin" />
			);
		case "failed":
			return <XCircle className="h-4 w-4 shrink-0 text-destructive" />;
		default:
			return <Circle className="h-4 w-4 shrink-0 text-muted-foreground" />;
	}
}

function formatTime(timestamp: number): string {
	const date = new Date(timestamp);
	return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

interface WorkflowTimelineProps {
	entries: TimelineEntry[];
}

export function WorkflowTimeline({ entries }: WorkflowTimelineProps) {
	if (entries.length === 0) {
		return (
			<div className="flex items-center justify-center h-full text-sm text-muted-foreground">
				No timeline entries
			</div>
		);
	}

	return (
		<ScrollArea className="h-full">
			<div className="flex flex-col gap-1 p-2">
				{entries.map((entry) => (
					<div
						key={entry.id}
						className="flex items-center gap-2 px-2 py-1.5 rounded text-sm"
					>
						<StatusIcon status={entry.status} />
						<span className="flex-1 truncate">{entry.label}</span>
						<span className="text-xs text-muted-foreground shrink-0">
							{formatTime(entry.timestamp)}
						</span>
					</div>
				))}
			</div>
		</ScrollArea>
	);
}
