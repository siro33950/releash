import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	Circle,
	Clock,
	Loader2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceStepStatus } from "@/types/workspace-tree";

export const workflowStepIconClasses: Record<WorkspaceStepStatus, string> = {
	queued: "text-muted-foreground",
	running: "text-blue-600 dark:text-blue-300",
	waiting_approval: "text-yellow-600 dark:text-yellow-300",
	completed: "text-green-600 dark:text-green-300",
	failed: "text-red-600 dark:text-red-300",
	aborted: "text-muted-foreground",
};

interface WorkflowStepStatusIconProps {
	status: WorkspaceStepStatus;
	containerClassName?: string;
	iconClassName?: string;
	circleClassName?: string;
}

export function WorkflowStepStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
	circleClassName = "size-2.5 shrink-0",
}: WorkflowStepStatusIconProps) {
	const colorClassName = workflowStepIconClasses[status];
	const inheritedColor = containerClassName ? undefined : colorClassName;
	const baseIconClassName = cn(iconClassName, inheritedColor);
	const icon =
		status === "running" ? (
			<Loader2 className={cn(baseIconClassName, "animate-spin")} />
		) : status === "completed" ? (
			<CheckCircle2 className={baseIconClassName} />
		) : status === "failed" ? (
			<AlertTriangle className={baseIconClassName} />
		) : status === "waiting_approval" ? (
			<Clock className={baseIconClassName} />
		) : status === "aborted" ? (
			<Ban className={baseIconClassName} />
		) : (
			<Circle className={cn(circleClassName, inheritedColor)} />
		);

	if (!containerClassName) {
		return icon;
	}

	return <span className={cn(containerClassName, colorClassName)}>{icon}</span>;
}
