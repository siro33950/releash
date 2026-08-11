export interface AgentSessionItem {
	id: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	lifecycle: "open" | "paused" | "archived";
	activity: "running" | "idle";
	lastExitAbnormal: boolean;
	operations: {
		canArchive: boolean;
		canRestore: boolean;
		canDelete: boolean;
	};
}

export interface AgentSessionLaunchAttachment {
	agentSessionId: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: AgentSessionItem["provider"];
}

export interface AgentSessionListPage {
	items: AgentSessionItem[];
	nextAfterSessionId: string | null;
}

export interface AgentSessionHistoryCandidate {
	provider: string;
	providerSessionId: string;
}

export interface AgentSessionHistoryPage {
	items: AgentSessionHistoryCandidate[];
	nextAfter: string | null;
}
