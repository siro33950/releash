export interface ProviderAgentSessionItem {
	id: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: string;
	origin:
		| { kind: "standalone" }
		| {
				kind: "workflow_node";
				workflow_execution_id: string;
				node_execution_id: string;
		  };
	lifecycle: "open" | "paused" | "archived";
	activity: "running" | "idle";
	lastExitAbnormal: boolean;
	providerSessionId: string | null;
	transcriptRef: string | null;
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
	updatedAtMs: number;
}

export interface ProviderAgentSessionHistoryPage {
	items: ProviderAgentSessionHistoryCandidate[];
	nextAfter: string | null;
}
