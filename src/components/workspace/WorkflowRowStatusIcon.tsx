import { Workflow } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceStepStatus } from "@/types/workspace-tree";
import { workflowStepIconClasses } from "./WorkflowStepStatusIcon";

const pulseStatuses: ReadonlySet<WorkspaceStepStatus> =
	new Set<WorkspaceStepStatus>(["running", "waiting"]);

interface WorkflowRowStatusIconProps {
	status: WorkspaceStepStatus;
	containerClassName?: string;
	iconClassName?: string;
}

export function WorkflowRowStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
}: WorkflowRowStatusIconProps) {
	const colorClassName =
		workflowStepIconClasses[status] ?? "text-muted-foreground";
	const pulseClassName = pulseStatuses.has(status)
		? "animate-pulse"
		: undefined;

	return (
		<span className={containerClassName} title={status}>
			<Workflow className={cn(iconClassName, colorClassName, pulseClassName)} />
		</span>
	);
}
