import type { PermissionMode } from "./session";

type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

interface TokenUsage {
	inputTokens: number;
	outputTokens: number;
}

/**
 * spec issues-1023: step / child の終端状態。
 * `"completed"` が既定（旧 ndjson 互換）。`"failed"` は partial child failure、
 * `"aborted"` は `RunAborted` で中断された step / parallel child を表現する。
 */
type StepEntryState = "completed" | "failed" | "aborted";

type FailureDisposition =
	| "retryable"
	| "partial"
	| "terminal"
	| "user-action-required";

interface ChildOutputSnapshot {
	stepName: string;
	sessionId?: string;
	result?: string;
	runIndex: number;
	completedAt: number;
	structuredOutput?: JsonValue;
	outputContract?: string;
	/** 受信側 optional。未指定時は `"completed"` 扱い（旧バックエンド互換）。 */
	state?: StepEntryState;
	failureKind?: WorkflowStepFailureKind;
	failureDisposition?: FailureDisposition;
}

interface StepHistoryEntry {
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

type WorkflowStepFailureKind =
	| "startup_timeout"
	| "stale_runtime_timeout"
	| "model_refusal"
	| "structured_output_mismatch"
	| "validation_failure"
	| "user_abort"
	| "infrastructure_crash";

type WorkflowExecutionState =
	| { type: "running" }
	| { type: "waiting_approval" }
	| { type: "completed" }
	| {
			type: "failed";
			reason: string;
			failureKind: WorkflowStepFailureKind;
			retryCount?: number;
	  }
	| { type: "aborted" };

interface TransitionRule {
	match: string;
	next: string;
}

interface CycleGuard {
	max_iterations: number;
}

// [02] Normalized Workflow: 旧 StepMode は廃止され、NodeType に統合された。
export type NodeType = "agent" | "bash" | "approval" | "parallel";

interface AggregateConfig {
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
interface ChildNodeDefinition {
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

interface CollectConfig {
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

interface StepOutput {
	stepName: string;
	runIndex: number;
	sessionId?: string;
	result?: string;
	structuredOutput?: JsonValue;
	outputContract?: string;
	tokenUsage?: TokenUsage;
	completedAt: number;
}

interface ParallelStepState {
	stepName: string;
	state: string;
	sessionId?: string;
	result?: string;
	runIndex: number;
	completedAt?: number;
	structuredOutput?: JsonValue;
	outputContract?: string;
	failureKind?: WorkflowStepFailureKind;
	failureDisposition?: FailureDisposition;
}

interface WorkflowStepRuntimeState {
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

interface ApprovalOperations {
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

type DiagnosticSeverity = "error" | "warning" | "info";

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

interface FacetUsageEntry {
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
