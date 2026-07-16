import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import {
	isWorkspaceNodePulseStatus,
	workflowNodeIconClasses,
} from "./WorkflowNodeStatusIcon";

interface WorkspaceBranchStatusIconProps {
	status: WorkspaceNodeStatus;
	icon: LucideIcon;
	containerClassName?: string;
	iconClassName?: string;
}

/** Workflow / Fanout branch 行の status アイコン。アイコン種だけが異なる。 */
export function WorkspaceBranchStatusIcon({
	status,
	icon: Icon,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
}: WorkspaceBranchStatusIconProps) {
	const colorClassName =
		workflowNodeIconClasses[status] ?? "text-muted-foreground";
	const pulseClassName = isWorkspaceNodePulseStatus(status)
		? "animate-pulse"
		: undefined;

	return (
		<span className={containerClassName} title={status}>
			<Icon className={cn(iconClassName, colorClassName, pulseClassName)} />
		</span>
	);
}
