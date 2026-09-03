export interface AgentSessionItem {
	id: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	treeLocation: {
		treeId: string;
		nodeExecutionId: string;
	};
	lifecycle: "open" | "paused" | "archived";
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
	label: string;
}

export interface AgentSessionHistoryPage {
	items: AgentSessionHistoryCandidate[];
	nextAfter: string | null;
}
