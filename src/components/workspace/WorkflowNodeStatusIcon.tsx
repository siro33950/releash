import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	Circle,
	Clock,
	Loader2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";

export const workflowNodeIconClasses: Record<WorkspaceNodeStatus, string> = {
	running: "text-blue-600 dark:text-blue-300",
	paused: "text-muted-foreground",
	failed: "text-red-600 dark:text-red-300",
	waiting: "text-yellow-600 dark:text-yellow-300",
	interrupted: "text-orange-600 dark:text-orange-300",
	aborted: "text-muted-foreground",
	completed: "text-green-600 dark:text-green-300",
};

/** running / waiting のときにアイコンを pulse させるかの判定。 */
export function isWorkspaceNodePulseStatus(
	status: WorkspaceNodeStatus,
): boolean {
	return status === "running" || status === "waiting";
}

interface WorkflowNodeStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
	circleClassName?: string;
}

export function WorkflowNodeStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
	circleClassName = "size-2.5 shrink-0",
}: WorkflowNodeStatusIconProps) {
	const colorClassName = workflowNodeIconClasses[status];
	const inheritedColor = containerClassName ? undefined : colorClassName;
	const baseIconClassName = cn(iconClassName, inheritedColor);
	const icon =
		status === "running" ? (
			<Loader2 className={cn(baseIconClassName, "animate-spin")} />
		) : status === "completed" ? (
			<CheckCircle2 className={baseIconClassName} />
		) : status === "failed" ? (
			<AlertTriangle className={baseIconClassName} />
		) : status === "waiting" ? (
			<Clock className={baseIconClassName} />
		) : status === "aborted" ? (
			<Ban className={baseIconClassName} />
		) : (
			<Circle className={cn(circleClassName, inheritedColor)} />
		);

	if (!containerClassName) {
		return icon;
	}

	return (
		<span className={cn(containerClassName, colorClassName)} title={status}>
			{icon}
		</span>
	);
}
