import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	ReviewErrorPayload,
	ReviewThread,
	WsMessage,
} from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteReviewThreadsOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
	connected: boolean;
	selectedWorktree: string | null;
}

export function useRemoteReviewThreads({
	subscribe,
	send,
	connected,
	selectedWorktree,
}: UseRemoteReviewThreadsOptions) {
	const [threads, setThreads] = useState<ReviewThread[]>([]);
	const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const worktreeName = selectedWorktree ?? null;
	const worktreeNameRef = useRef(worktreeName);
	worktreeNameRef.current = worktreeName;

	useEffect(() => {
		if (selectedWorktree == null) {
			setLoading(false);
		}
		setThreads([]);
		setSelectedThreadId(null);
		setError(null);
	}, [selectedWorktree]);

	const refresh = useCallback(() => {
		if (!connected || !worktreeName) return;
		setLoading(true);
		setError(null);
		send({
			type: "review_list_request",
			payload: { worktreeName, filter: null },
		});
	}, [connected, worktreeName, send]);

	useEffect(() => {
		refresh();
	}, [refresh]);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "review_list_response") {
				if (msg.payload.worktreeName !== worktreeNameRef.current) return;
				setLoading(false);
				if (msg.payload.success) {
					setThreads(msg.payload.threads);
					setError(null);
					setSelectedThreadId((current) => {
						if (current && msg.payload.threads.some((t) => t.id === current)) {
							return current;
						}
						return msg.payload.threads[0]?.id ?? null;
					});
				} else {
					setError(formatReviewError(msg.payload.error));
				}
			}
			if (msg.type === "review_thread_response") {
				if (msg.payload.worktreeName !== worktreeNameRef.current) return;
				const thread = msg.payload.thread;
				if (msg.payload.success && thread) {
					setThreads((current) => upsertThread(current, thread));
					setSelectedThreadId(thread.id);
					setError(null);
				} else {
					setError(formatReviewError(msg.payload.error));
				}
			}
		});
	}, [subscribe]);

	const selectedThread = useMemo(
		() => threads.find((thread) => thread.id === selectedThreadId) ?? null,
		[threads, selectedThreadId],
	);

	const createThread = useCallback(
		(content: string) => {
			if (!worktreeName) return;
			send({
				type: "review_create_request",
				payload: {
					worktreeName,
					target: {},
					content,
				},
			});
		},
		[worktreeName, send],
	);

	const appendComment = useCallback(
		(threadId: string, content: string) => {
			if (!worktreeName) return;
			send({
				type: "review_append_comment_request",
				payload: {
					worktreeName,
					threadId,
					content,
				},
			});
		},
		[worktreeName, send],
	);

	const resolveThread = useCallback(
		(threadId: string, summary: string) => {
			if (!worktreeName) return;
			send({
				type: "review_resolve_request",
				payload: {
					worktreeName,
					threadId,
					outcome: "resolved",
					summary,
				},
			});
		},
		[worktreeName, send],
	);

	return {
		threads,
		selectedThread,
		selectedThreadId,
		loading,
		error,
		setSelectedThreadId,
		refresh,
		createThread,
		appendComment,
		resolveThread,
	};
}

function upsertThread(
	threads: ReviewThread[],
	incoming: ReviewThread,
): ReviewThread[] {
	const next = threads.filter((thread) => thread.id !== incoming.id);
	next.unshift(incoming);
	return next.sort((a, b) => b.updatedAt - a.updatedAt);
}

function formatReviewError(error?: ReviewErrorPayload | null): string | null {
	if (!error) return null;
	return `${error.code}: ${error.message}`;
}
