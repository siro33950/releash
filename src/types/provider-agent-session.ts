export interface ProviderAgentSessionItem {
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

export interface ProviderAgentSessionLaunchAttachment {
	agentSessionId: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: ProviderAgentSessionItem["provider"];
}

export interface ProviderAgentSessionListPage {
	items: ProviderAgentSessionItem[];
	nextAfterSessionId: string | null;
}

export interface ProviderAgentSessionHistoryCandidate {
	provider: string;
	providerSessionId: string;
}

export interface ProviderAgentSessionHistoryPage {
	items: ProviderAgentSessionHistoryCandidate[];
	nextAfter: string | null;
}
