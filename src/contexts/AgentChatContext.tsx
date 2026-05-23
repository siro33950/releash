import { createContext, useContext, useEffect, useMemo } from "react";
import { type UseAgentChatResult, useAgentChat } from "@/hooks/useAgentChat";
import { useWorkflowState } from "@/hooks/useWorkflowState";

/**
 * spec issues-1023: `useAgentChat` を MainLayout レベルに引き上げ、AgentChatPanel と
 * WorkflowSidebarPanel が同一の reducer state（session store / streaming / activity）を
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
	// approval-chat の run_id 解決は AgentChatPanel 個別の責務ではなく、
	// 現 worktree の workflow state から導出されるグローバルな観測。Provider 側で
	// 一度だけ解決し、useAgentChat に注入する。parent ChatSession 機構は撤去済み
	// であり、approval chat の宛先は step session (`currentSessionId`) のみ。
	const { workflowState } = useWorkflowState(worktreePath);
	const workflowApprovalChatSessionId =
		workflowState?.state.type === "waiting_approval"
			? (workflowState.currentSessionId ?? null)
			: null;
	const workflowApprovalRunId =
		workflowState?.state.type === "waiting_approval"
			? (workflowState.executionId ?? null)
			: null;

	const agentChat = useAgentChat(
		worktreePath,
		workflowApprovalChatSessionId,
		workflowApprovalRunId,
	);

	const { sessions, refreshSessions, refreshClosedSessions } = agentChat;

	// 既存挙動: workflow state が新規 step session を露出したら session 一覧を refresh。
	const knownWorkflowSessionIds = useMemo(() => {
		return new Set(sessions.map((session) => session.id));
	}, [sessions]);
	const workflowStateUpdatedAt = workflowState?.updatedAt;

	useEffect(() => {
		const workflowSessionIds = [workflowState?.currentSessionId].filter(
			(id): id is string => Boolean(id),
		);

		if (
			workflowSessionIds.some(
				(sessionId) => !knownWorkflowSessionIds.has(sessionId),
			)
		) {
			refreshSessions();
		}
	}, [
		workflowState?.currentSessionId,
		knownWorkflowSessionIds,
		refreshSessions,
	]);

	useEffect(() => {
		if (workflowStateUpdatedAt == null) return;
		refreshSessions({ reconcileActiveSession: true });
		refreshClosedSessions();
	}, [workflowStateUpdatedAt, refreshSessions, refreshClosedSessions]);

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
