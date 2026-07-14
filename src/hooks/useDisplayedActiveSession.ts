import { useMemo } from "react";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import type { ChatSession } from "@/types/session";

/**
 * AgentChat の active session のうち、ユーザーに「現在の Agent との対話」として
 * 提示してよい session を返す。workflow node として起動された session は
 * AgentChat 本文タブに並ばないため除外する (AgentChatPanel と同じ判定規則)。
 *
 * spec issues-1022 "Thread handoff contract": Diff Thread を Agent に共有する
 * 操作の送信先判定は、AgentChat 表示と同じ規則で導出する。
 *
 * @returns active session が存在し、かつ workflow node session でない場合はその session。
 *          いずれかの条件を満たさない場合は `null` (= 送信不可状態)。
 */
export function useDisplayedActiveSession(): ChatSession | null {
	const { activeSession } = useAgentChatContext();
	return useMemo(() => {
		if (!activeSession) return null;
		if (activeSession.workflowNodeSession) return null;
		return activeSession;
	}, [activeSession]);
}
