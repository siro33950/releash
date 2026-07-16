import { GitFork } from "lucide-react";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";
import { WorkspaceBranchStatusIcon } from "./WorkspaceBranchStatusIcon";

interface FanoutRowStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
}

export function FanoutRowStatusIcon(props: FanoutRowStatusIconProps) {
	return <WorkspaceBranchStatusIcon {...props} icon={GitFork} />;
}
