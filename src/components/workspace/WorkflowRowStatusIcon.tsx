import { Workflow } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";

const pulseStatuses: ReadonlySet<WorkspaceNodeStatus> =
	new Set<WorkspaceNodeStatus>(["running", "waiting"]);

interface WorkflowRowStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
}

export function WorkflowRowStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
}: WorkflowRowStatusIconProps) {
	const colorClassName =
		workflowNodeIconClasses[status] ?? "text-muted-foreground";
	const pulseClassName = pulseStatuses.has(status)
		? "animate-pulse"
		: undefined;

	return (
		<span className={containerClassName} title={status}>
			<Workflow className={cn(iconClassName, colorClassName, pulseClassName)} />
		</span>
	);
}
