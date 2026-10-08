import { GitFork } from "lucide-react";
import type { WorkspaceNodeStatusClassification } from "@/types/workspace-tree";
import { WorkspaceBranchStatusIcon } from "./WorkspaceBranchStatusIcon";

interface FanoutRowStatusIconProps {
	status: WorkspaceNodeStatusClassification;
	containerClassName?: string;
	iconClassName?: string;
}

export function FanoutRowStatusIcon(props: FanoutRowStatusIconProps) {
	return <WorkspaceBranchStatusIcon {...props} icon={GitFork} />;
}
