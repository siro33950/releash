import type { AgentState, AgentStateSync } from "@/types/protocol";

const STATE_PRIORITY: Record<AgentState, number> = {
	error: 4,
	waiting: 3,
	running: 2,
	done: 1,
};

export function aggregateAgentState(
	states: Map<string, AgentStateSync>,
	worktreePath: string,
): AgentState | undefined {
	let best: AgentState | undefined;
	let bestPriority = 0;

	for (const entry of states.values()) {
		if (entry.worktree_path !== worktreePath) continue;
		const p = STATE_PRIORITY[entry.state] ?? 0;
		if (p > bestPriority) {
			bestPriority = p;
			best = entry.state;
		}
	}

	return best;
}

export function highestPriorityState(
	states: AgentState[],
): AgentState | undefined {
	let best: AgentState | undefined;
	let bestPriority = 0;
	for (const s of states) {
		const p = STATE_PRIORITY[s] ?? 0;
		if (p > bestPriority) {
			bestPriority = p;
			best = s;
		}
	}
	return best;
}

export function aggregateFromEntries(
	entries: AgentStateSync[],
): AgentState | undefined {
	return highestPriorityState(entries.map((e) => e.state));
}

export function agentStateKey(
	worktreePath: string,
	ptyId?: string | null,
): string {
	return ptyId ? `${worktreePath}::${ptyId}` : worktreePath;
}
