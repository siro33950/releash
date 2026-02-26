import { useCallback, useEffect, useRef, useState } from "react";
import { agentStateKey, aggregateAgentState } from "@/lib/agentStateUtils";
import type { AgentState, AgentStateSync } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseAgentStateOptions {
	subscribe: Subscribe;
}

export function useAgentState({ subscribe }: UseAgentStateOptions) {
	const [agentStates, setAgentStates] = useState<Map<string, AgentStateSync>>(
		new Map(),
	);
	const statesRef = useRef(agentStates);
	statesRef.current = agentStates;

	const getAgentState = useCallback(
		(worktreePath: string): AgentState | undefined => {
			return aggregateAgentState(statesRef.current, worktreePath);
		},
		[],
	);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "agent_state_sync") {
				setAgentStates((prev) => {
					const key = agentStateKey(
						msg.payload.worktree_path,
						msg.payload.pty_id,
					);
					const next = new Map(prev);
					next.set(key, msg.payload);
					return next;
				});
			}
		});
	}, [subscribe]);

	return { agentStates, getAgentState };
}
