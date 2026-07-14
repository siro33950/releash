import { invoke } from "@tauri-apps/api/core";
import { createContext, useContext, useMemo } from "react";
import { useAgentChatContext } from "@/contexts/AgentChatContext";

/**
 * spec issues-1022 "Thread handoff contract" / "Human-to-agent thread handoff flow":
 * Diff Thread を現在の Agent との対話に共有する操作を、UI 階層の深さに依存せず
 * 取り出せるようにする context。worktreeName は MainLayout 層で 1 度だけ解決し、
 * 子孫の DiffInlineComment / DiffCommentList は context 経由で worktreeName 無しの
 * シグネチャで sendThreadToAgent を呼べる。
 *
 * - メッセージ本文の整形は Rust 側 (`build_review_thread_handoff` Tauri command) が owner
 * - 送信先 session は AgentChat の active session そのもの。workflow node session を
 *   開いている場合でも、その session 自身に送付できる (ユーザーが「現在開いている Agent」
 *   に対するハンドオフ操作として観測する)。
 * - active session が存在しない場合 `canSend` が `false` となり、UI 側はボタンを disabled にする
 */
export interface ReviewThreadHandoffContextValue {
	canSend: boolean;
	sendThreadToAgent: (threadId: string) => Promise<void>;
}

/**
 * Provider 配下ではない context を差し込めるよう export する (テスト用)。
 * 通常は `ReviewThreadHandoffProvider` を使う。
 */
const ReviewThreadHandoffContext =
	createContext<ReviewThreadHandoffContextValue | null>(null);

interface ProviderProps {
	worktreeName: string;
	children: React.ReactNode;
}

export function ReviewThreadHandoffProvider({
	worktreeName,
	children,
}: ProviderProps) {
	const { activeSession, sendMessage } = useAgentChatContext();

	const value = useMemo<ReviewThreadHandoffContextValue>(() => {
		const activeSessionId = activeSession?.id ?? null;
		return {
			canSend: activeSessionId !== null,
			sendThreadToAgent: async (threadId: string) => {
				if (activeSessionId === null) {
					return;
				}
				const content = await invoke<string>("build_review_thread_handoff", {
					worktreeName,
					threadId,
				});
				await sendMessage(activeSessionId, content);
			},
		};
	}, [activeSession, sendMessage, worktreeName]);

	return (
		<ReviewThreadHandoffContext.Provider value={value}>
			{children}
		</ReviewThreadHandoffContext.Provider>
	);
}

/**
 * Diff Thread を Active な AgentChat session に共有する handler を返す。
 *
 * Provider 配下でない場合は `canSend === false` & 何もしない `sendThreadToAgent` を返す
 * (UI 上は disabled として観測される)。これによりテストや provider 外コンポーネントでも
 * 安全にレンダリングできる。
 */
export function useReviewThreadHandoff(): ReviewThreadHandoffContextValue {
	const ctx = useContext(ReviewThreadHandoffContext);
	if (ctx) return ctx;
	return {
		canSend: false,
		sendThreadToAgent: async () => {
			/* no-op outside provider */
		},
	};
}
