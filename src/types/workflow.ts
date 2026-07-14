import type { PermissionMode } from "./session";

export type JsonValue =
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
 * `"aborted"` / `"interrupted"` は run の中断で停止した step / fanout child を表現する。
 */
type StepEntryState = "completed" | "failed" | "aborted" | "interrupted";

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
	artifactContract?: string;
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
	| { type: "aborted" }
	| { type: "interrupted" };

type Rule =
	| {
			type: "when";
			on: string;
			then: string;
			next: string;
	  }
	| {
			type: "switch";
			on: string;
			cases: Record<string, string>;
			next?: string;
	  }
	| {
			type: "loop_guard";
			max_iterations: number;
			on_exhausted: string;
	  }
	| {
			type: "next";
			next: string;
	  };

export type NodeKind = "command" | "session" | "fanout";
export type SessionGate = "auto" | "approval";

export interface FacetRefs {
	policy?: string;
	knowledge?: string;
	instruction?: string;
}

export interface SessionSpec {
	model?: string;
	permission?: PermissionMode;
	gate: SessionGate;
	facets: FacetRefs;
}

export type WorkflowSchema = JsonValue;

export type FanoutItemsSource = JsonValue[] | string;

interface FanoutSpec {
	child: string[];
	items?: FanoutItemsSource;
}

export interface NodeDefinition {
	name: string;
	kind: NodeKind;
	command?: string;
	session?: SessionSpec;
	fanout?: FanoutSpec;
	artifact?: string;
	input?: string;
	inputs?: string[];
	// 共通: rules は省略時 undefined（Rust 側で serde default 経路を持つが、frontend
	// fixture では空配列を毎回書かなくて済むよう optional とする）
	rules?: Rule[];
}

export interface Workflow {
	name: string;
	description: string;
	builtin: boolean;
	schemas?: Record<string, WorkflowSchema>;
	nodes: NodeDefinition[];
}

interface StepOutput {
	stepName: string;
	runIndex: number;
	sessionId?: string;
	result?: string;
	structuredOutput?: JsonValue;
	artifactContract?: string;
	tokenUsage?: TokenUsage;
	completedAt: number;
}

export type NodeExecutionStatus =
	| "running"
	| "waiting_approval"
	| "succeeded"
	| "failed"
	| "aborted";

export interface FanoutParentRef {
	parentNode: string;
	parentAttempt: number;
	itemIndex?: number;
	childIndex: number;
}

export interface NodeExecutionFailure {
	reason: string;
	kind: WorkflowStepFailureKind;
}

export interface NodeExecutionView {
	id: string;
	executionId: string;
	nodeName: string;
	kind: NodeKind;
	attempt: number;
	status: NodeExecutionStatus;
	sessionId?: string;
	artifact?: JsonValue;
	tokenUsage?: TokenUsage;
	failure?: NodeExecutionFailure;
	fanoutParent?: FanoutParentRef;
	startedAt: number;
	completedAt?: number;
}

interface WorkflowStepRuntimeState {
	runtimeActive: boolean;
	tabOpen: boolean;
}

export interface WorkflowStallObservation {
	chatSessionId: string;
	stepName: string;
	runIndex: number;
	turnPhase: string;
	idleSecs: number;
	signalCount: number;
	capReached: boolean;
	observedAt: number;
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
	workflowDefinition: Workflow;
	totalTokenUsage: TokenUsage;
	stepStates: Record<string, string>;
	runtimeStates?: Record<string, WorkflowStepRuntimeState>;
	nodeExecutions: NodeExecutionView[];
	startedAt: number;
	updatedAt: number;
	stallObservations?: WorkflowStallObservation[];
}

/// Workflow run 一覧コマンドから返る
/// WorkflowRun のサマリ表現。Rust 側 `WorkflowRunSummary` のフィールドに対応する（camelCase）。
export interface WorkflowRunSummary {
	runId: string;
	workflowName: string;
	task?: string | null;
	status:
		| "running"
		| "waiting_approval"
		| "completed"
		| "failed"
		| "aborted"
		| "interrupted";
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

export type FacetKind = "policy" | "knowledge" | "instruction";

export interface FacetSummary {
	key: string;
	kind: string;
	description: string;
	builtin: boolean;
}

type DiagnosticSeverity = "error" | "warning" | "info";
type DiagnosticStage = "parse_shape" | "resolve" | "typecheck" | "control_flow";

interface DiagnosticSpan {
	start_line: number;
	start_col: number;
	end_line: number;
	end_col: number;
}

export interface DiagnosticItem {
	code: string;
	severity: DiagnosticSeverity;
	stage: DiagnosticStage;
	span?: DiagnosticSpan;
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

export type SaveWorkflowSourceResponse =
	| {
			ok: true;
			workflow: Workflow;
			diagnostics?: DiagnosticItem[];
			error?: string;
	  }
	| {
			ok: false;
			workflow?: null;
			diagnostics: DiagnosticItem[];
			error?: string;
	  };
