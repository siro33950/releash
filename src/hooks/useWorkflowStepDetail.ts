import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { JsonValue, TokenUsage } from "@/types/workflow";

/**
 * spec issues-1023: Rust 側 StepDetailView を frontend で受け取るための型。
 * 入出力・遷移結果・所要時間（および static な input facts）を engine 側 projection
 * が一括で返す境界。frontend は表示用フォーマットのみを担う。
 */
export interface WorkflowStepInputView {
	instruction?: string;
	policy?: string;
	knowledge?: string;
	outputContract?: string;
	inputContracts?: string[];
	previousStepName?: string;
	previousStepStructuredOutput?: JsonValue;
}

export interface WorkflowStepDetailView {
	stepName: string;
	nodeType: string;
	runIndex: number;
	state: string;
	sessionId?: string;
	result?: string;
	structuredOutput?: JsonValue;
	tokenUsage?: TokenUsage;
	startedAtMs?: number;
	completedAtMs?: number;
	durationMs?: number;
	input: WorkflowStepInputView;
}

export interface UseWorkflowStepDetailInput {
	worktreePath: string | null;
	runId: string | null;
	nodeName: string | null;
	runIndex?: number;
}

export interface UseWorkflowStepDetailResult {
	detail: WorkflowStepDetailView | null;
	isLoading: boolean;
	error: string | null;
}

export function useWorkflowStepDetail(
	input: UseWorkflowStepDetailInput,
): UseWorkflowStepDetailResult {
	const { worktreePath, runId, nodeName, runIndex } = input;
	const [detail, setDetail] = useState<WorkflowStepDetailView | null>(null);
	const [isLoading, setIsLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		if (!worktreePath || !runId || !nodeName) {
			setDetail(null);
			setError(null);
			setIsLoading(false);
			return;
		}
		let cancelled = false;
		setIsLoading(true);
		setError(null);
		invoke<WorkflowStepDetailView | null>("get_workflow_step_detail", {
			worktreePath,
			runId,
			nodeName,
			runIndex: runIndex ?? null,
		})
			.then((result) => {
				if (cancelled) return;
				setDetail(result ?? null);
			})
			.catch((e) => {
				console.warn(
					"[useWorkflowStepDetail] get_workflow_step_detail failed",
					e,
				);
				if (!cancelled) {
					setDetail(null);
					setError(String(e));
				}
			})
			.finally(() => {
				if (!cancelled) setIsLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [worktreePath, runId, nodeName, runIndex]);

	return { detail, isLoading, error };
}
