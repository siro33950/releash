import type { WorkspaceNodeStatusClassification } from "@/types/workspace-tree";

export const workflowNodeIconClasses: Record<
	WorkspaceNodeStatusClassification,
	string
> = {
	active: "text-blue-600 dark:text-blue-300",
	attention: "text-yellow-600 dark:text-yellow-300",
	idle: "text-green-600 dark:text-green-300",
};

export function isWorkspaceNodePulseStatus(
	status: WorkspaceNodeStatusClassification,
): boolean {
	return status === "active" || status === "attention";
}
