import { invoke } from "@tauri-apps/api/core";
import {
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";

export interface ReviewThreadHandoffFeedback {
	threadId: string;
	kind: "copied" | "error";
	message: string;
}

export interface ReviewThreadHandoffContextValue {
	canCopy: boolean;
	feedback: ReviewThreadHandoffFeedback | null;
	copyThreadForAgent: (threadId: string) => Promise<void>;
}

export function ReviewThreadHandoffFeedbackMessage({
	feedback,
	threadId,
}: {
	feedback: ReviewThreadHandoffFeedback | null;
	threadId: string;
}) {
	if (feedback?.threadId !== threadId) return null;
	return (
		<span
			className={`max-w-64 truncate rounded px-1.5 py-0.5 text-[10px] ${
				feedback.kind === "error"
					? "bg-destructive/10 text-destructive"
					: "bg-muted text-muted-foreground"
			}`}
			role={feedback.kind === "error" ? "alert" : "status"}
			title={feedback.message}
		>
			{feedback.message}
		</span>
	);
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
	const [feedback, setFeedback] = useState<ReviewThreadHandoffFeedback | null>(
		null,
	);
	const copyThreadForAgent = useCallback(
		async (threadId: string) => {
			try {
				const content = await invoke<string>("build_review_thread_handoff", {
					worktreeName,
					threadId,
				});
				await navigator.clipboard.writeText(content);
				setFeedback({
					threadId,
					kind: "copied",
					message: "Agent instruction copied",
				});
			} catch (error) {
				setFeedback({
					threadId,
					kind: "error",
					message: `Failed to copy Agent instruction: ${String(error)}`,
				});
			}
		},
		[worktreeName],
	);

	const value = useMemo<ReviewThreadHandoffContextValue>(() => {
		return {
			canCopy: true,
			feedback,
			copyThreadForAgent,
		};
	}, [copyThreadForAgent, feedback]);

	return (
		<ReviewThreadHandoffContext.Provider value={value}>
			{children}
		</ReviewThreadHandoffContext.Provider>
	);
}

export function useReviewThreadHandoff(): ReviewThreadHandoffContextValue {
	const ctx = useContext(ReviewThreadHandoffContext);
	if (ctx) return ctx;
	return {
		canCopy: false,
		feedback: null,
		copyThreadForAgent: async () => {
			/* no-op outside provider */
		},
	};
}
