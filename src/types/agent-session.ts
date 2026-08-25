export interface AgentSessionItem {
	id: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	treeParent?: {
		treeId: string;
		nodeExecutionId: string;
	} | null;
	lifecycle: "open" | "paused" | "archived";
	activity: "running" | "idle";
	lastExitAbnormal: boolean;
	operations: {
		canArchive: boolean;
		canRestore: boolean;
		canDelete: boolean;
		canResume: boolean;
	};
}

export interface AgentSessionLaunchAttachment {
	agentSessionId: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: AgentSessionItem["provider"];
}

export interface AgentSessionHistoryCandidate {
	provider: string;
	providerSessionId: string;
}

export interface AgentSessionHistoryPage {
	items: AgentSessionHistoryCandidate[];
	nextAfter: string | null;
}
