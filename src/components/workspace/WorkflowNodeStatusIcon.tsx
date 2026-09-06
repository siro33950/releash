import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	Circle,
	CircleHelp,
	Clock,
	Loader2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type {
	WorkspaceNodeStatus,
	WorkspaceNodeStatusClassification,
} from "@/types/workspace-tree";

export const workflowNodeIconClasses: Record<
	WorkspaceNodeStatusClassification,
	string
> = {
	active: "text-blue-600 dark:text-blue-300",
	attention: "text-yellow-600 dark:text-yellow-300",
	failure: "text-red-600 dark:text-red-300",
	idle: "text-green-600 dark:text-green-300",
	unbound: "text-muted-foreground",
};

export function isWorkspaceNodePulseStatus(
	status: WorkspaceNodeStatusClassification,
): boolean {
	return status === "active" || status === "attention";
}

interface WorkflowNodeStatusIconProps {
	status: WorkspaceNodeStatus;
	statusClassification: WorkspaceNodeStatusClassification;
	containerClassName?: string;
	iconClassName?: string;
	circleClassName?: string;
}

export function WorkflowNodeStatusIcon({
	status,
	statusClassification,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
	circleClassName = "size-2.5 shrink-0",
}: WorkflowNodeStatusIconProps) {
	const colorClassName = workflowNodeIconClasses[statusClassification];
	const inheritedColor = containerClassName ? undefined : colorClassName;
	const baseIconClassName = cn(iconClassName, inheritedColor);
	const icon =
		status === "unresolved" ? (
			<CircleHelp className={baseIconClassName} />
		) : status === "running" ? (
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
