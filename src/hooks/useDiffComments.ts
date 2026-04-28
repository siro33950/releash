import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { DiffComment } from "@/types/diffComment";
import type { MentionReference } from "@/types/session";

interface UseDiffCommentsOptions {
	worktreeName: string;
}

export function useDiffComments({ worktreeName }: UseDiffCommentsOptions) {
	const [comments, setComments] = useState<DiffComment[]>([]);
	const [loading, setLoading] = useState(false);
	const worktreeNameRef = useRef(worktreeName);
	worktreeNameRef.current = worktreeName;

	const loadComments = useCallback(async () => {
		if (!worktreeName) return;
		const requestedWorktree = worktreeName;
		setLoading(true);
		try {
			const result = await invoke<DiffComment[]>("load_diff_comments", {
				worktreeName: requestedWorktree,
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
		const unlisten = listen<string>("diff-comments-changed", (event) => {
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
			filePath: string;
			lineNumber?: number;
			endLine?: number;
			content: string;
		}) => {
			return invoke<DiffComment>("add_diff_comment", {
				worktreeName,
				filePath: params.filePath,
				lineNumber: params.lineNumber ?? null,
				endLine: params.endLine ?? null,
				content: params.content,
			});
		},
		[worktreeName],
	);

	const updateComment = useCallback(
		async (commentId: string, content: string) => {
			return invoke<void>("update_diff_comment", {
				worktreeName,
				commentId,
				content,
			});
		},
		[worktreeName],
	);

	const deleteComment = useCallback(
		async (commentId: string) => {
			return invoke<void>("delete_diff_comment", {
				worktreeName,
				commentId,
			});
		},
		[worktreeName],
	);

	const sendToAgent = useCallback(
		async (commentIds: string[]) => {
			return invoke<{
				sentCount: number;
				formattedMessage: string;
				mentions: MentionReference[];
				commentIds: string[];
			}>("send_diff_comments_to_agent", {
				worktreeName,
				commentIds,
			});
		},
		[worktreeName],
	);

	const markSent = useCallback(
		async (commentIds: string[]) => {
			return invoke<void>("mark_diff_comments_sent", {
				worktreeName,
				commentIds,
			});
		},
		[worktreeName],
	);

	const sendAllUnsent = useCallback(async () => {
		return sendToAgent([]);
	}, [sendToAgent]);

	const getCommentsForFile = useCallback(
		(filePath: string) => {
			return comments.filter((c) => c.filePath === filePath);
		},
		[comments],
	);

	const unsentCount = comments.filter((c) => c.status === "unsent").length;

	return {
		comments,
		loading,
		unsentCount,
		addComment,
		updateComment,
		deleteComment,
		sendToAgent,
		markSent,
		sendAllUnsent,
		getCommentsForFile,
		reload: loadComments,
	};
}
