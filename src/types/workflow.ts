export type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

export interface TokenUsage {
	inputTokens: number;
	outputTokens: number;
}

export interface ChildOutputSnapshot {
	stepName: string;
	sessionId?: string;
	result?: string;
	runIndex: number;
	completedAt: number;
	structuredOutput?: JsonValue;
	outputContract?: string;
}

export interface StepHistoryEntry {
	stepName: string;
	completedAt: number;
	result: string | null;
	sessionId?: string;
	tokenUsage?: TokenUsage;
	structuredOutput?: JsonValue;
	runIndex?: number;
	childOutputs?: ChildOutputSnapshot[];
}

export type WorkflowExecutionState =
	| { type: "running" }
	| { type: "waiting_approval" }
	| { type: "completed" }
	| { type: "failed"; reason: string }
	| { type: "aborted" };

export interface TransitionRule {
	match: string;
	next: string;
}

export interface CycleGuard {
	max_iterations: number;
}

export type StepMode = "auto" | "approval";

export interface ParallelStep {
	name: string;
	mode: StepMode;
	policy?: string;
	knowledge?: string;
	instruction?: string;
	output_contract?: string;
	pass_previous_response?: boolean;
	pass_output_from?: string[];
}

export interface AggregateConfig {
	all_match?: string;
	any_match?: string;
	then: string;
	else: string;
}

export interface Step {
	name: string;
	mode?: StepMode;
	policy?: string;
	knowledge?: string;
	instruction?: string;
	output_contract?: string;
	rules: TransitionRule[];
	cycle_guard?: CycleGuard;
	pass_previous_response?: boolean;
	pass_output_from?: string[];
	inline_prompt?: string;
	collect?: CollectConfig;
	parallel?: ParallelStep[];
	aggregate?: AggregateConfig;
}

export interface CollectConfig {
	from: string[];
	reduce: ReduceStrategy;
}

export type ReduceStrategy =
	| "last"
	| "concat"
	| "grouped"
	| "any_needs_fix"
	| "all_passed";

export interface Workflow {
	name: string;
	description: string;
	builtin: boolean;
	steps: Step[];
}

export interface StepOutput {
	stepName: string;
	runIndex: number;
	sessionId?: string;
	result?: string;
	structuredOutput?: JsonValue;
	outputContract?: string;
	tokenUsage?: TokenUsage;
	completedAt: number;
}

export interface ParallelStepState {
	stepName: string;
	state: string;
	sessionId?: string;
	result?: string;
	runIndex: number;
	completedAt?: number;
	structuredOutput?: JsonValue;
	outputContract?: string;
}

export interface WorkflowState {
	executionId: string;
	workflowName: string;
	chatSessionId?: string;
	state: WorkflowExecutionState;
	currentStepIndex: number;
	currentStepName: string;
	currentSessionId?: string;
	totalSteps: number;
	stepHistory: StepHistoryEntry[];
	stepExecutionCounts: Record<string, number>;
	stepOutputs: Record<string, StepOutput>;
	workflowVariables?: Record<string, string>;
	workflowDefinition: Workflow;
	totalTokenUsage: TokenUsage;
	stepStates: Record<string, string>;
	activeParallelSteps?: ParallelStepState[];
	startedAt: number;
	updatedAt: number;
	approvalOperations?: ApprovalOperations;
}

export interface ApprovalOperations {
	canReject: boolean;
}

export interface WorkflowStatePayload {
	worktreePath: string;
	workflowState: WorkflowState;
}

export type WorkflowLogEvent =
	| {
			event: "workflow_started";
			execution_id: string;
			workflow_name: string;
			workflow_file_stem?: string;
			worktree_path: string;
			timestamp: number;
	  }
	| {
			event: "step_started";
			execution_id: string;
			workflow_name: string;
			step_name: string;
			execution_count: number;
			timestamp: number;
	  }
	| {
			event: "step_completed";
			execution_id: string;
			workflow_name: string;
			step_name: string;
			result: string | null;
			session_id?: string;
			token_usage?: TokenUsage;
			structured_output?: JsonValue;
			run_index?: number;
			timestamp: number;
	  }
	| {
			event: "step_failed";
			execution_id: string;
			workflow_name: string;
			step_name: string;
			reason: string;
			timestamp: number;
	  }
	| {
			event: "workflow_completed";
			execution_id: string;
			workflow_name: string;
			total_token_usage: TokenUsage;
			timestamp: number;
	  }
	| {
			event: "workflow_failed";
			execution_id: string;
			workflow_name: string;
			reason: string;
			timestamp: number;
	  }
	| {
			event: "workflow_aborted";
			execution_id: string;
			workflow_name: string;
			timestamp: number;
	  }
	| {
			event: "output_collected";
			execution_id: string;
			workflow_name: string;
			step_name: string;
			step_outputs: CollectedOutputEntry[];
			reduce_strategy: string;
			reduce_result?: string;
			reduce_structured_output?: JsonValue;
			timestamp: number;
	  }
	| {
			event: "contract_repair_requested";
			execution_id: string;
			workflow_name: string;
			step_name: string;
			attempt: number;
			violation_reason: string;
			timestamp: number;
	  }
	| {
			event: "parallel_started";
			execution_id: string;
			workflow_name: string;
			parent_step_name: string;
			child_step_names: string[];
			timestamp: number;
	  }
	| {
			event: "parallel_step_started";
			execution_id: string;
			workflow_name: string;
			parent_step_name: string;
			child_step_name: string;
			session_id: string;
			execution_count: number;
			timestamp: number;
	  }
	| {
			event: "parallel_step_completed";
			execution_id: string;
			workflow_name: string;
			parent_step_name: string;
			child_step_name: string;
			result: string | null;
			session_id: string;
			token_usage?: TokenUsage;
			structured_output?: JsonValue;
			run_index: number;
			timestamp: number;
	  }
	| {
			event: "parallel_completed";
			execution_id: string;
			workflow_name: string;
			parent_step_name: string;
			aggregate_result: string;
			timestamp: number;
	  };

export interface CollectedOutputEntry {
	stepName: string;
	result?: string;
	structuredOutput?: JsonValue;
}

export type WorkflowSummary = {
	name: string;
	description: string;
	builtin: boolean;
	is_running: boolean;
};

export type FacetKind =
	| "policy"
	| "knowledge"
	| "instruction"
	| "output_contract";

export interface FacetSummary {
	key: string;
	kind: string;
	description: string;
	builtin: boolean;
}

export type DiagnosticSeverity = "error" | "warning" | "info";

export interface DiagnosticItem {
	severity: DiagnosticSeverity;
	message: string;
	workflow_name?: string;
	step_name?: string;
	facet_key?: string;
	facet_kind?: string;
	field?: string;
}

export interface DiagnosticSummary {
	error_count: number;
	warning_count: number;
	info_count: number;
}

export interface FacetUsageEntry {
	workflow_name: string;
	step_name: string;
	slot: string;
}

export interface DiagnosticReport {
	items: DiagnosticItem[];
	workflow_summaries: Record<string, DiagnosticSummary>;
	facet_summaries: Record<string, DiagnosticSummary>;
	facet_usage: Record<string, FacetUsageEntry[]>;
}
