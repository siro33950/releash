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

export type NodeExecutionFailureKind =
	| "startup_timeout"
	| "stale_runtime_timeout"
	| "model_refusal"
	| "structured_output_mismatch"
	| "validation_failure"
	| "user_abort"
	| "infrastructure_crash";

export type WorkflowExecutionStatus =
	| "running"
	| "waiting_approval"
	| "completed"
	| "aborted"
	| "interrupted";

export type ExecutionInterruptionReason = "crash" | "stale" | "stop" | "orphan";

export type Rule =
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
			reset_on?: string;
	  }
	| {
			type: "next";
			next: string;
	  };

export type NodeKind = "command" | "session" | "fanout" | "sequence";
export type NodeCompletion = "auto" | "approval";

export interface FacetRefs {
	policy?: string;
	knowledge?: string[];
	instruction?: string;
}

export interface SessionSpec {
	provider: "claude" | "codex";
	model?: string;
	permission?: string;
	facets: FacetRefs;
}

export interface InputParam {
	name: string;
	contract?: string;
}

export type SchemaDefView = JsonValue;

export type FanoutItemsSource = JsonValue[] | string;

export interface ChildInput {
	parameter: string;
	source: string;
}

export interface ChildEntry {
	name: string;
	inputs?: ChildInput[];
	// 省略 = 隣接辺 auto（リストの次へ）、空配列 = 明示終端
	rules?: Rule[];
}

export interface FanoutSpec {
	children: ChildEntry[];
	items?: FanoutItemsSource;
}

export interface SequenceSpec {
	entry?: string;
	output?: string;
	children: ChildEntry[];
}

export interface NodeDefinition {
	name: string;
	kind: NodeKind;
	command?: string;
	session?: SessionSpec;
	fanout?: FanoutSpec;
	sequence?: SequenceSpec;
	artifact?: string;
	input?: InputParam[];
	completion?: NodeCompletion;
	worktree?: string;
}

export interface WorkflowDefinition {
	name: string;
	description: string;
	builtin: boolean;
	schemas?: Record<string, SchemaDefView>;
	nodes: NodeDefinition[];
}

export type NodeExecutionStatus =
	| "running"
	| "paused"
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
	kind: NodeExecutionFailureKind;
}

export interface NodeExecution {
	id: string;
	executionId: string;
	nodeName: string;
	kind: NodeKind;
	attempt: number;
	status: NodeExecutionStatus;
	submitReceived: boolean;
	stopReceived: boolean;
	waitingFor?: "submit" | "stop";
	canApprove: boolean;
	canRetry: boolean;
	hasArtifact: boolean;
	sessionId?: string;
	artifact?: Artifact;
	tokenUsage?: TokenUsage;
	failure?: NodeExecutionFailure;
	fanoutParent?: FanoutParentRef;
	startedAt: number;
	completedAt?: number;
}

export interface Artifact {
	nodeName: string;
	contract?: string;
	value: JsonValue;
	producedAt: number;
}

export interface Fanout {
	parent: NodeExecution;
	children: NodeExecution[];
	artifact?: Artifact;
}

export interface ApprovalTarget {
	nodeExecutionId: string;
	nodeName: string;
	sessionId?: string;
}

export interface WorkflowExecution {
	id: string;
	workflowName: string;
	status: WorkflowExecutionStatus;
	currentNode?: string | null;
	worktreePath: string;
	createdFrom: "desktop_ui" | "cli" | "agent" | "api";
	startedAt: number;
	updatedAt: number;
	completedAt?: number | null;
	errorReason?: string | null;
	interruptionReason?: ExecutionInterruptionReason | null;
	resumeFromNode?: string | null;
	totalTokenUsage: TokenUsage;
	nodeExecutions: NodeExecution[];
	artifacts: Artifact[];
	fanouts: Fanout[];
	approvalTarget?: ApprovalTarget | null;
}

export interface WorkflowExecutionSummary {
	executionId: string;
	workflowName: string;
	status: WorkflowExecutionStatus;
	worktreePath: string;
	currentNode?: string | null;
	createdFrom: "desktop_ui" | "cli" | "agent" | "api";
	startedAt: number;
	updatedAt: number;
	completedAt?: number | null;
	errorReason?: string | null;
	interruptionReason?: ExecutionInterruptionReason | null;
	resumeFromNode?: string | null;
	totalTokenUsage: TokenUsage;
}

export interface WorkflowExecutionChangedPayload {
	worktreePath: string;
	workflowExecution: WorkflowExecution;
}

export type WorkflowDefinitionSummary = {
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

type DiagnosticSeverity = "error" | "info";
type DiagnosticStage = "parse_shape" | "resolve" | "typecheck" | "control_flow";

interface DiagnosticSpan {
	start_line: number;
	start_col: number;
	end_line: number;
	end_col: number;
}

export interface DiagnosticView {
	code: string;
	severity: DiagnosticSeverity;
	stage: DiagnosticStage;
	span?: DiagnosticSpan;
	message: string;
	workflow_name?: string;
	node_name?: string;
	facet_key?: string;
	facet_kind?: string;
	field?: string;
}

export interface DiagnosticSummary {
	error_count: number;
	info_count: number;
}

interface FacetUsageEntry {
	workflow_name: string;
	node_name: string;
	slot: string;
}

export interface DiagnosticReport {
	items: DiagnosticView[];
	workflow_summaries: Record<string, DiagnosticSummary>;
	facet_summaries: Record<string, DiagnosticSummary>;
	facet_usage: Record<string, FacetUsageEntry[]>;
}

export type SaveWorkflowSourceResponse =
	| {
			ok: true;
			workflow: WorkflowDefinition;
			diagnostics?: DiagnosticView[];
			error?: string;
	  }
	| {
			ok: false;
			workflow?: null;
			diagnostics: DiagnosticView[];
			error?: string;
	  };
