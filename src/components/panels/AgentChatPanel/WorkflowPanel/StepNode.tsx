import { Handle, type NodeProps, Position } from "@xyflow/react";
import type { StepMode } from "@/types/workflow";

export interface StepNodeData {
	label: string;
	mode: StepMode;
	state:
		| "pending"
		| "running"
		| "completed"
		| "failed"
		| "waiting_approval"
		| "aborted";
	executionCount: number;
	isCurrent: boolean;
	[key: string]: unknown;
}

const stateColors: Record<StepNodeData["state"], string> = {
	running: "border-blue-500 bg-blue-500/10",
	completed: "border-green-500 bg-green-500/10",
	failed: "border-red-500 bg-red-500/10",
	waiting_approval: "border-yellow-500 bg-yellow-500/10",
	pending: "border-muted-foreground/30 bg-muted/50",
	aborted: "border-muted-foreground bg-muted",
};

export function StepNode({ data }: NodeProps) {
	const nodeData = data as unknown as StepNodeData;
	const colorClass = stateColors[nodeData.state];

	return (
		<div
			className={`px-3 py-2 rounded-md border-2 min-w-[180px] text-center ${colorClass} ${nodeData.isCurrent ? "ring-2 ring-primary" : ""}`}
		>
			<Handle
				type="target"
				position={Position.Top}
				className="!bg-muted-foreground/50 !w-2 !h-2"
			/>
			<div className="text-sm font-medium">{nodeData.label}</div>
			<div className="text-xs text-muted-foreground mt-0.5">
				{nodeData.mode}
				{nodeData.executionCount > 0 && (
					<span className="ml-1 px-1 rounded bg-muted text-muted-foreground">
						×{nodeData.executionCount}
					</span>
				)}
			</div>
			<Handle
				type="source"
				position={Position.Bottom}
				className="!bg-muted-foreground/50 !w-2 !h-2"
			/>
		</div>
	);
}
