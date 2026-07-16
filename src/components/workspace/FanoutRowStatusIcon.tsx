import { GitFork } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { workflowNodeIconClasses } from "./WorkflowNodeStatusIcon";

const pulseStatuses: ReadonlySet<WorkspaceNodeStatus> =
	new Set<WorkspaceNodeStatus>(["running", "waiting"]);

interface FanoutRowStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
}

export function FanoutRowStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
}: FanoutRowStatusIconProps) {
	const colorClassName =
		workflowNodeIconClasses[status] ?? "text-muted-foreground";
	const pulseClassName = pulseStatuses.has(status)
		? "animate-pulse"
		: undefined;

	return (
		<span className={containerClassName} title={status}>
			<GitFork className={cn(iconClassName, colorClassName, pulseClassName)} />
		</span>
	);
}
