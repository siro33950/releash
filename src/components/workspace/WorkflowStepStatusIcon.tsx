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
	failed: "text-red-600 dark:text-red-300",
	error: "text-destructive",
	waiting: "text-yellow-600 dark:text-yellow-300",
	aborted: "text-muted-foreground",
	completed: "text-green-600 dark:text-green-300",
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
		) : status === "failed" || status === "error" ? (
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
