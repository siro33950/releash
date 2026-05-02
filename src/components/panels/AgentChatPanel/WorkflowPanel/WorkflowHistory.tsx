import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { WorkflowLogEvent, WorkflowState } from "@/types/workflow";
import { StepDetail } from "./StepDetail";
import { WorkflowGraph } from "./WorkflowGraph";

export function WorkflowHistory({
	worktreePath,
	workflowState: currentWorkflowState,
}: {
	worktreePath: string;
	workflowState?: WorkflowState | null;
}) {
	const [executionIds, setExecutionIds] = useState<string[]>([]);
	const [selectedId, setSelectedId] = useState<string | null>(null);
	const [events, setEvents] = useState<WorkflowLogEvent[]>([]);
	const [historyState, setHistoryState] = useState<WorkflowState | null>(null);
	const [selectedStep, setSelectedStep] = useState<string | null>(null);
	const selectionReqRef = useRef(0);

	const fetchExecutionIds = useCallback(() => {
		invoke<string[]>("list_workflow_executions", { worktreePath })
			.then(setExecutionIds)
			.catch((e) =>
				console.warn("[WorkflowHistory] list_workflow_executions failed", e),
			);
	}, [worktreePath]);

	// 初期取得
	useEffect(() => {
		fetchExecutionIds();
	}, [fetchExecutionIds]);

	// ワークフロー完了時に履歴一覧を再取得
	const stateType = currentWorkflowState?.state.type;
	useEffect(() => {
		if (
			stateType === "completed" ||
			stateType === "failed" ||
			stateType === "aborted"
		) {
			fetchExecutionIds();
		}
	}, [stateType, fetchExecutionIds]);

	const selectExecution = useCallback((id: string) => {
		const reqId = ++selectionReqRef.current;
		setSelectedId(id);
		setSelectedStep(null);
		setEvents([]);
		setHistoryState(null);
		invoke<WorkflowLogEvent[]>("get_workflow_execution_log", {
			executionId: id,
		})
			.then((nextEvents) => {
				if (selectionReqRef.current === reqId) {
					setEvents(nextEvents);
				}
			})
			.catch((e) =>
				console.warn("[WorkflowHistory] get_workflow_execution_log failed", e),
			);
		invoke<WorkflowState | null>("get_workflow_execution_state", {
			executionId: id,
		})
			.then((nextState) => {
				if (selectionReqRef.current === reqId) {
					setHistoryState(nextState ?? null);
				}
			})
			.catch((e) =>
				console.warn(
					"[WorkflowHistory] get_workflow_execution_state failed",
					e,
				),
			);
	}, []);

	if (executionIds.length === 0) {
		return (
			<div className="px-3 py-2 text-xs text-muted-foreground">
				No execution history
			</div>
		);
	}

	return (
		<div className="flex flex-col gap-1">
			<div className="px-3 py-1 text-xs font-medium text-muted-foreground">
				Past executions
			</div>
			{executionIds.map((id) => (
				<button
					key={id}
					type="button"
					className={`px-3 py-1 text-xs text-left hover:bg-muted/50 ${selectedId === id ? "bg-muted" : ""}`}
					onClick={() => selectExecution(id)}
				>
					{id}
				</button>
			))}
			{selectedId && historyState && (
				<div className="border-t">
					{(historyState.totalTokenUsage.inputTokens > 0 ||
						historyState.totalTokenUsage.outputTokens > 0) && (
						<div className="px-3 py-1 text-xs text-muted-foreground">
							Total:{" "}
							{historyState.totalTokenUsage.inputTokens +
								historyState.totalTokenUsage.outputTokens}{" "}
							tokens
						</div>
					)}
					<div className="h-[200px]">
						<WorkflowGraph
							workflowState={historyState}
							onStepClick={setSelectedStep}
						/>
					</div>
					{selectedStep && (
						<div className="border-t">
							<div className="flex items-center justify-between px-3 py-1 border-b">
								<span className="text-xs font-medium">{selectedStep}</span>
								<button
									type="button"
									className="text-xs text-muted-foreground hover:text-foreground"
									onClick={() => setSelectedStep(null)}
								>
									Close
								</button>
							</div>
							<StepDetail
								stepName={selectedStep}
								workflowState={historyState}
							/>
						</div>
					)}
				</div>
			)}
			{selectedId && events.length > 0 && (
				<div className="px-3 py-1 border-t">
					{events.map((event) => (
						<div
							key={`${event.event}-${event.timestamp}`}
							className="text-xs text-muted-foreground py-0.5"
						>
							<span className="font-mono">{event.event}</span>
							{"step_name" in event && event.step_name && (
								<span className="ml-1">({event.step_name})</span>
							)}
						</div>
					))}
				</div>
			)}
		</div>
	);
}
