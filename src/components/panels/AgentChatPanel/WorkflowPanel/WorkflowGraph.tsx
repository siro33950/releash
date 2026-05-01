import {
	type Edge,
	type Node,
	type NodeTypes,
	ReactFlow,
	type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { useCallback, useEffect, useMemo, useRef } from "react";
import type { WorkflowState } from "@/types/workflow";
import { StepNode, type StepNodeData } from "./StepNode";

const nodeTypes: NodeTypes = {
	step: StepNode,
};

const NODE_WIDTH = 180;
const NODE_HEIGHT = 60;
const NODE_GAP = 40;

export function WorkflowGraph({
	workflowState,
	onStepClick,
}: {
	workflowState: WorkflowState;
	onStepClick?: (stepName: string) => void;
}) {
	const instanceRef = useRef<ReactFlowInstance<
		Node<StepNodeData>,
		Edge
	> | null>(null);
	const fitViewOpts = useMemo(
		() => ({ padding: 0.1, minZoom: 1, maxZoom: 1 }),
		[],
	);

	const onInit = useCallback(
		(instance: ReactFlowInstance<Node<StepNodeData>, Edge>) => {
			instanceRef.current = instance;
			requestAnimationFrame(() => {
				instance.fitView(fitViewOpts);
			});
		},
		[fitViewOpts],
	);

	// Re-fit when workflow state changes
	// biome-ignore lint/correctness/useExhaustiveDependencies: workflowState triggers re-fit on step transitions
	useEffect(() => {
		if (!instanceRef.current) return;
		requestAnimationFrame(() => {
			instanceRef.current?.fitView(fitViewOpts);
		});
	}, [workflowState, fitViewOpts]);

	const { nodes, edges } = useMemo(() => {
		const steps = workflowState.workflowDefinition.steps;
		const builtNodes: Node<StepNodeData>[] = steps.map((step, index) => ({
			id: step.name,
			type: "step",
			position: { x: 0, y: index * (NODE_HEIGHT + NODE_GAP) },
			width: NODE_WIDTH,
			height: NODE_HEIGHT,
			data: {
				label: step.name,
				mode: step.mode,
				state: (workflowState.stepStates[step.name] ??
					"pending") as StepNodeData["state"],
				executionCount: workflowState.stepExecutionCounts[step.name] ?? 0,
				isCurrent: index === workflowState.currentStepIndex,
			},
		}));

		const builtEdges: Edge[] = [];
		for (let i = 0; i < steps.length - 1; i++) {
			builtEdges.push({
				id: `${steps[i].name}->${steps[i + 1].name}`,
				source: steps[i].name,
				target: steps[i + 1].name,
				animated:
					i === workflowState.currentStepIndex &&
					workflowState.state.type === "running",
			});
		}
		// Transition rule edges
		for (const step of steps) {
			for (const rule of step.rules) {
				const edgeId = `${step.name}->${rule.next}[${rule.match}]`;
				if (!builtEdges.some((e) => e.id === edgeId)) {
					builtEdges.push({
						id: edgeId,
						source: step.name,
						target: rule.next,
						label: rule.match,
						style: { strokeDasharray: "5,5" },
					});
				}
			}
		}

		return { nodes: builtNodes, edges: builtEdges };
	}, [workflowState]);

	return (
		<div className="h-full w-full">
			<ReactFlow
				nodes={nodes}
				edges={edges}
				nodeTypes={nodeTypes}
				onInit={onInit}
				fitView
				fitViewOptions={fitViewOpts}
				minZoom={1}
				maxZoom={1}
				panOnScroll
				panOnDrag={false}
				zoomOnScroll={false}
				zoomOnPinch={false}
				zoomOnDoubleClick={false}
				nodesDraggable={false}
				nodesConnectable={false}
				elementsSelectable={false}
				onNodeClick={(_event, node) => onStepClick?.(node.id)}
				proOptions={{ hideAttribution: true }}
			/>
		</div>
	);
}
