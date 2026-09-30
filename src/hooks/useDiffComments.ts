import { useCallback } from "react";
import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";
import { invokeClient as invoke } from "@/lib/client";
import { rethrowClientError } from "@/lib/clientErrorNotice";
import { getThreadFilePath } from "@/types/diffComment";

interface UseDiffCommentsOptions {
	worktreeName: string;
}

export function useDiffComments({ worktreeName }: UseDiffCommentsOptions) {
	const subscription = useStateSubscriptionResult(
		worktreeName ? { kind: "review-threads", args: [worktreeName] } : null,
	);
	const comments = subscription.value ?? [];

	const addComment = useCallback(
		async (params: {
			filePath?: string;
			lineNumber?: number;
			endLine?: number;
			content: string;
		}) => {
			return invoke("create_review_thread", {
				worktreeName,
				filePath: params.filePath ?? null,
				lineNumber: params.lineNumber ?? null,
				endLine: params.endLine ?? null,
				content: params.content,
			}).catch(rethrowClientError);
		},
		[worktreeName],
	);

	const appendComment = useCallback(
		async (threadId: string, content: string) => {
			await invoke("append_review_comment", {
				worktreeName,
				threadId,
				content,
			}).catch(rethrowClientError);
		},
		[worktreeName],
	);

	const resolveThread = useCallback(
		async (threadId: string, outcome: string, summary: string) => {
			await invoke("resolve_review_thread", {
				worktreeName,
				threadId,
				outcome,
				summary,
			}).catch(rethrowClientError);
		},
		[worktreeName],
	);

	const deleteThread = useCallback(
		async (threadId: string) => {
			await invoke("delete_review_thread", {
				worktreeName,
				threadId,
			}).catch(rethrowClientError);
		},
		[worktreeName],
	);

	const getCommentsForFile = useCallback(
		(filePath: string) => {
			return comments.filter(
				(thread) => getThreadFilePath(thread) === filePath,
			);
		},
		[comments],
	);

	return {
		comments,
		loading: Boolean(
			worktreeName && subscription.value === undefined && !subscription.error,
		),
		error: subscription.error,
		unsentCount: 0,
		addComment,
		appendComment,
		resolveThread,
		deleteThread,
		getCommentsForFile,
	};
}
