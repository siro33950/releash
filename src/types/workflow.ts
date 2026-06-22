import type { PermissionMode } from "./session";

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

/**
 * spec issues-1023: step / child の終端状態。
 * `"completed"` が既定（旧 ndjson 互換）。`"aborted"` は `RunAborted` で
 * 中断された step / parallel child を表現する。
 */
export type StepEntryState = "completed" | "aborted";

export interface ChildOutputSnapshot {
	stepName: string;
	sessionId?: string;
	result?: string;
	runIndex: number;
	completedAt: number;
	structuredOutput?: JsonValue;
	outputContract?: string;
	/** 受信側 optional。未指定時は `"completed"` 扱い（旧バックエンド互換）。 */
	state?: StepEntryState;
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
	/** 受信側 optional。未指定時は `"completed"` 扱い（旧バックエンド互換）。 */
	state?: StepEntryState;
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

// [02] Normalized Workflow: 旧 StepMode は廃止され、NodeType に統合された。
export type NodeType = "agent" | "bash" | "approval" | "parallel";

export interface AggregateConfig {
	all_match?: string;
	any_match?: string;
	then: string;
	else: string;
}

/// 並列 node 配下の子 node の API 表現。
///
/// [02] schema 境界: Rust 側 `ChildNodeDefinition` と語彙を一致させるため、子 node には
/// top-level 専用フィールド（`rules` / `cycle_guard` / `resets_cycle_for` / `collect` /
/// `parallel_children` / `aggregate` / `command`）を持たせない。
export interface ChildNodeDefinition {
	name: string;
	type: NodeType;
	policy?: string;
	knowledge?: string;
	instruction?: string;
	output_contract?: string;
	input_contracts?: string[];
	pass_previous_response?: boolean;
	pass_output_from?: string[];
	model?: string;
	permission?: PermissionMode;
}

/// node 種別ごとの設定は、boundary doc では agent_config / approval_config /
/// command_config / parallel_children として概念分類されるが、frontend では
/// 表示・編集の都合上フラットなフィールドで保持する。Rust schema 側もフラット。
export interface NodeDefinition {
	name: string;
	type: NodeType;
	// agent / approval 種別で使用される prompt 関連 facet 参照
	policy?: string;
	knowledge?: string;
	instruction?: string;
	output_contract?: string;
	input_contracts?: string[];
	pass_previous_response?: boolean;
	pass_output_from?: string[];
	inline_prompt?: string;
	collect?: CollectConfig;
	// bash 種別で使用される command
	command?: string;
	// parallel 種別で使用される子 node 群と集約条件
	parallel_children?: ChildNodeDefinition[];
	aggregate?: AggregateConfig;
	// 共通: rules は省略時 undefined（Rust 側で serde default 経路を持つが、frontend
	// fixture では空配列を毎回書かなくて済むよう optional とする）
	rules?: TransitionRule[];
	cycle_guard?: CycleGuard;
	resets_cycle_for?: string[];
	model?: string;
	permission?: PermissionMode;
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
	nodes: NodeDefinition[];
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

export interface WorkflowStepRuntimeState {
	runtimeActive: boolean;
	tabOpen: boolean;
}

export interface WorkflowState {
	executionId: string;
	workflowName: string;
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
	runtimeStates?: Record<string, WorkflowStepRuntimeState>;
	activeParallelSteps?: ParallelStepState[];
	startedAt: number;
	updatedAt: number;
	approvalOperations?: ApprovalOperations;
}

export interface ApprovalOperations {
	canReject: boolean;
}

/// Workflow run 一覧コマンドから返る
/// WorkflowRun のサマリ表現。Rust 側 `WorkflowRunSummary` のフィールドに対応する（camelCase）。
export interface WorkflowRunSummary {
	runId: string;
	workflowName: string;
	task?: string | null;
	status: "running" | "waiting_approval" | "completed" | "failed" | "aborted";
	worktreePath: string;
	currentNodeName?: string | null;
	triggerSource: "desktop_ui" | "remote" | "cli" | "agent";
	startedAt: number;
	updatedAt: number;
	completedAt?: number | null;
	errorReason?: string | null;
}

export interface WorkflowStatePayload {
	worktreePath: string;
	workflowState: WorkflowState;
}

/// spec issues-1023: backend `WorkflowEventView` の TypeScript 表現。
///
/// engine 側 domain `WorkflowEvent.timestamp` は秒単位 f64 だが、frontend 観測経路は
/// projection 境界で `WorkflowEventView` に変換される（`timestampMs` /
/// `requestedAtMs` で単位を明示）。`WorkflowEvent` という型名はこの view 型を指す。
///
/// NDJSON tag は snake_case（`run_started` / `node_started` 等）。`run_id` を主語とし、
/// node 識別子は `node_name` で表す。表示用フォーマット以外のロジックはここに追加しない
/// （rust-first-logic 準拠）。
export type WorkflowEvent =
	| {
			event: "run_started";
			run_id: string;
			workflow_name: string;
			workflow_file_stem: string;
			worktree_path: string;
			workflow_definition: Workflow;
			timestampMs: number;
	  }
	| {
			event: "node_started";
			run_id: string;
			workflow_name: string;
			node_name: string;
			execution_count: number;
			timestampMs: number;
	  }
	| {
			event: "step_session_started";
			run_id: string;
			workflow_name: string;
			node_name: string;
			session_id: string;
			execution_count: number;
			timestampMs: number;
	  }
	| {
			event: "node_completed";
			run_id: string;
			workflow_name: string;
			node_name: string;
			result?: string;
			session_id?: string;
			token_usage?: TokenUsage;
			structured_output?: JsonValue;
			run_index?: number;
			timestampMs: number;
	  }
	| {
			event: "node_failed";
			run_id: string;
			workflow_name: string;
			node_name: string;
			reason: string;
			timestampMs: number;
	  }
	| {
			event: "approval_requested";
			run_id: string;
			workflow_name: string;
			node_name: string;
			timestampMs: number;
	  }
	| {
			event: "approval_resolved";
			run_id: string;
			workflow_name: string;
			node_name: string;
			decision: "approve" | "reject" | "abort";
			comment?: string;
			timestampMs: number;
	  }
	| {
			event: "run_completed";
			run_id: string;
			workflow_name: string;
			total_token_usage: TokenUsage;
			timestampMs: number;
	  }
	| {
			event: "run_failed";
			run_id: string;
			workflow_name: string;
			reason: string;
			timestampMs: number;
	  }
	| {
			event: "run_aborted";
			run_id: string;
			workflow_name: string;
			timestampMs: number;
	  }
	| {
			event: "cli_mutation_requested";
			run_id: string;
			workflow_name: string;
			request_id: string;
			request: CliMutationRequestRecord;
			requestedAtMs: number;
			timestampMs: number;
	  }
	| {
			event: "output_collected";
			run_id: string;
			workflow_name: string;
			node_name: string;
			node_outputs: CollectedOutputEntry[];
			reduce_strategy: string;
			reduce_result?: string;
			reduce_structured_output?: JsonValue;
			timestampMs: number;
	  }
	| {
			event: "contract_repair_requested";
			run_id: string;
			workflow_name: string;
			node_name: string;
			attempt: number;
			violation_reason: string;
			timestampMs: number;
	  }
	| {
			event: "parallel_started";
			run_id: string;
			workflow_name: string;
			parent_node_name: string;
			child_node_names: string[];
			timestampMs: number;
	  }
	| {
			event: "parallel_child_started";
			run_id: string;
			workflow_name: string;
			parent_node_name: string;
			child_node_name: string;
			session_id: string;
			execution_count: number;
			timestampMs: number;
	  }
	| {
			event: "parallel_child_completed";
			run_id: string;
			workflow_name: string;
			parent_node_name: string;
			child_node_name: string;
			result?: string;
			session_id: string;
			token_usage?: TokenUsage;
			structured_output?: JsonValue;
			run_index: number;
			timestampMs: number;
	  }
	| {
			event: "parallel_completed";
			run_id: string;
			workflow_name: string;
			parent_node_name: string;
			aggregate_result: string;
			timestampMs: number;
	  };

export type CliMutationRequestRecord =
	| {
			kind: "approve";
			node_name?: string | null;
			comment?: string | null;
	  }
	| {
			kind: "reject";
			node_name?: string | null;
			reason: string;
	  }
	| {
			kind: "abort";
			node_name?: string | null;
	  };

export interface CollectedOutputEntry {
	nodeName: string;
	result?: string;
	structuredOutput?: JsonValue;
}

export type WorkflowSummary = {
	name: string;
	description: string;
	builtin: boolean;
	is_running: boolean;
};

export type FacetKind = "policy" | "knowledge" | "instruction" | "contract";

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
