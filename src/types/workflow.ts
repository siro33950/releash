export interface TokenUsage {
	inputTokens: number;
	outputTokens: number;
}

export interface StepHistoryEntry {
	stepName: string;
	completedAt: number;
	result: string | null;
	sessionId?: string;
	tokenUsage?: TokenUsage;
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
	maxIterations: number;
}

export type StepMode = "auto" | "approval" | "interactive";

export type StepPrompt = string | { inline: string } | { template: string };

export interface Step {
	name: string;
	mode: StepMode;
	prompt: StepPrompt;
	rules: TransitionRule[];
	cycleGuard?: CycleGuard;
}

export interface Workflow {
	name: string;
	description: string;
	builtin: boolean;
	steps: Step[];
}

export interface WorkflowState {
	executionId: string;
	workflowName: string;
	state: WorkflowExecutionState;
	currentStepIndex: number;
	currentStepName: string;
	totalSteps: number;
	stepHistory: StepHistoryEntry[];
	stepExecutionCounts: Record<string, number>;
	workflowDefinition: Workflow;
	totalTokenUsage: TokenUsage;
	stepStates: Record<string, string>;
	startedAt: number;
	updatedAt: number;
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
	  };
