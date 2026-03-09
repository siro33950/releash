export type WorkflowPhase =
	| "requirements"
	| "planning"
	| "implementation"
	| "review"
	| "completed";

export type TimelineEntryStatus =
	| "pending"
	| "in_progress"
	| "completed"
	| "failed";

export interface TimelineEntry {
	id: string;
	label: string;
	status: TimelineEntryStatus;
	timestamp: number;
}

export interface WorkflowState {
	phase: WorkflowPhase;
	timeline: TimelineEntry[];
	planDocument: string | null;
}
