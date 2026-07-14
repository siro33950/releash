import { createContext, useContext, useEffect, useMemo } from "react";
import { type UseAgentChatResult, useAgentChat } from "@/hooks/useAgentChat";
import { useWorkflowState } from "@/hooks/useWorkflowState";

/**
 * spec issues-1023: `useAgentChat` を MainLayout レベルに引き上げ、AgentChatPanel と
 * WorkflowView が同一の reducer state（session store / streaming / activity）を
 * Context 経由で共有するための provider。両 panel で別々に `useAgentChat` を呼び出すと
 * useReducer state が分離して破綻するため、必ず本 provider でラップして使う。
 */
const AgentChatContext = createContext<UseAgentChatResult | null>(null);

interface AgentChatProviderProps {
	worktreePath: string;
	children: React.ReactNode;
}

export function AgentChatProvider({
	worktreePath,
	children,
}: AgentChatProviderProps) {
	// Approval target の選択は backend-owned read model が所有する。frontend は
	// target を推測せず、返された NodeExecution / Session 参照をそのまま使う。
	const { workflowExecution } = useWorkflowState(worktreePath);
	const approvalTarget = workflowExecution?.approvalTarget ?? null;
	const workflowApprovalChatSessionId = approvalTarget?.sessionId ?? null;
	const workflowApprovalExecutionId = approvalTarget
		? (workflowExecution?.id ?? null)
		: null;

	const agentChat = useAgentChat(
		worktreePath,
		workflowApprovalChatSessionId,
		workflowApprovalExecutionId,
	);

	const { sessions, refreshSessions, refreshClosedSessions } = agentChat;

	// read model が新規 node session を露出したら session 一覧を refresh。
	const knownWorkflowSessionIds = useMemo(() => {
		return new Set(sessions.map((session) => session.id));
	}, [sessions]);
	const workflowExecutionUpdatedAt = workflowExecution?.updatedAt;

	useEffect(() => {
		const workflowSessionIds = [approvalTarget?.sessionId].filter(
			(id): id is string => Boolean(id),
		);

		if (
			workflowSessionIds.some(
				(sessionId) => !knownWorkflowSessionIds.has(sessionId),
			)
		) {
			refreshSessions();
		}
	}, [approvalTarget?.sessionId, knownWorkflowSessionIds, refreshSessions]);

	useEffect(() => {
		if (workflowExecutionUpdatedAt == null) return;
		refreshSessions({ reconcileActiveSession: true });
		refreshClosedSessions();
	}, [workflowExecutionUpdatedAt, refreshSessions, refreshClosedSessions]);

	// 安定参照のため、shallow に memo する（agentChat の中身は内部で memoize 済み）。
	const value = useMemo(() => agentChat, [agentChat]);

	return (
		<AgentChatContext.Provider value={value}>
			{children}
		</AgentChatContext.Provider>
	);
}

export function useAgentChatContext(): UseAgentChatResult {
	const ctx = useContext(AgentChatContext);
	if (!ctx) {
		throw new Error(
			"useAgentChatContext must be used within <AgentChatProvider>",
		);
	}
	return ctx;
}
