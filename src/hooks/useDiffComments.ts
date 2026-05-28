import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	getThreadFilePath,
	type ReviewDiscussionThread,
} from "@/types/diffComment";
import type { ReviewStanceValue, ReviewThread } from "@/types/protocol";

interface UseDiffCommentsOptions {
	worktreeName: string;
}

export function useDiffComments({ worktreeName }: UseDiffCommentsOptions) {
	const [comments, setComments] = useState<ReviewDiscussionThread[]>([]);
	const [loading, setLoading] = useState(false);
	const worktreeNameRef = useRef(worktreeName);
	worktreeNameRef.current = worktreeName;

	const loadComments = useCallback(async () => {
		if (!worktreeName) return;
		const requestedWorktree = worktreeName;
		setLoading(true);
		try {
			const result = await invoke<ReviewThread[]>("list_review_threads", {
				worktreeName: requestedWorktree,
				filter: null,
			});
			if (worktreeNameRef.current === requestedWorktree) {
				setComments(result ?? []);
			}
		} finally {
			setLoading(false);
		}
	}, [worktreeName]);

	useEffect(() => {
		loadComments();
	}, [loadComments]);

	useEffect(() => {
		const unlisten = listen<string>("review-comments-changed", (event) => {
			if (event.payload === worktreeNameRef.current) {
				loadComments();
			}
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	}, [loadComments]);

	const addComment = useCallback(
		async (params: {
			filePath?: string;
			lineNumber?: number;
			endLine?: number;
			content: string;
		}) => {
			return invoke<ReviewThread>("create_review_thread", {
				worktreeName,
				filePath: params.filePath ?? null,
				lineNumber: params.lineNumber ?? null,
				endLine: params.endLine ?? null,
				content: params.content,
			});
		},
		[worktreeName],
	);

	const appendComment = useCallback(
		async (
			threadId: string,
			content: string,
			stance?: ReviewStanceValue | null,
		) => {
			await invoke<ReviewThread>("append_review_comment", {
				worktreeName,
				threadId,
				content,
				stance: stance ?? null,
			});
		},
		[worktreeName],
	);

	const resolveThread = useCallback(
		async (threadId: string, outcome: string, summary: string) => {
			await invoke<ReviewThread>("resolve_review_thread", {
				worktreeName,
				threadId,
				outcome,
				summary,
			});
		},
		[worktreeName],
	);

	const deleteThread = useCallback(
		async (threadId: string) => {
			await invoke<void>("delete_review_thread", {
				worktreeName,
				threadId,
			});
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
		loading,
		unsentCount: 0,
		addComment,
		appendComment,
		resolveThread,
		deleteThread,
		getCommentsForFile,
		reload: loadComments,
	};
}
