import { Workflow } from "lucide-react";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { WorkspaceBranchStatusIcon } from "./WorkspaceBranchStatusIcon";

interface WorkflowRowStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
}

export function WorkflowRowStatusIcon(props: WorkflowRowStatusIconProps) {
	return <WorkspaceBranchStatusIcon {...props} icon={Workflow} />;
}
