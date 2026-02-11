import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentStateSync } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseAgentStateOptions {
	subscribe: Subscribe;
}

export function useAgentState({ subscribe }: UseAgentStateOptions) {
	const [agentStates, setAgentStates] = useState<
		Map<string, AgentStateSync>
	>(new Map());
	const statesRef = useRef(agentStates);
	statesRef.current = agentStates;

	const getAgentState = useCallback(
		(worktreePath: string): AgentStateSync | undefined => {
			return statesRef.current.get(worktreePath);
		},
		[],
	);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "agent_state_sync") {
				setAgentStates((prev) => {
					const next = new Map(prev);
					next.set(msg.payload.worktree_path, msg.payload);
					return next;
				});
			}
		});
	}, [subscribe]);

	return { agentStates, getAgentState };
}
