import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
	CommentSeverity,
	CommentTarget,
	LineComment,
} from "@/types/comment";
import {
	type CommentItemDTO,
	dtoToLineComment,
	lineCommentToDTO,
} from "@/types/comment";

export function useLineComments(worktreeName: string) {
	const [comments, setComments] = useState<LineComment[]>([]);
	const worktreeNameRef = useRef(worktreeName);
	worktreeNameRef.current = worktreeName;

	// Initial load
	useEffect(() => {
		let disposed = false;
		setComments([]);
		invoke<CommentItemDTO[]>("load_comments", { worktreeName }).then(
			(dtos) => {
				if (!disposed) setComments(dtos.map(dtoToLineComment));
			},
			(err) => {
				if (!disposed) console.error("Failed to load comments:", err);
			},
		);
		return () => {
			disposed = true;
		};
	}, [worktreeName]);

	// Listen for external changes (MCP, remote, etc.)
	useEffect(() => {
		const unlisten = listen<{ worktree_name: string; source: string }>(
			"comments-changed",
			(event) => {
				if (event.payload.worktree_name !== worktreeNameRef.current) return;
				if (event.payload.source === "desktop") return;
				const currentWorktree = worktreeNameRef.current;
				invoke<CommentItemDTO[]>("load_comments", {
					worktreeName: currentWorktree,
				}).then(
					(dtos) => {
						if (worktreeNameRef.current !== currentWorktree) return;
						setComments(dtos.map(dtoToLineComment));
					},
					(err) => {
						if (worktreeNameRef.current !== currentWorktree) return;
						console.error("Failed to reload comments:", err);
					},
				);
			},
		);
		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	const addComment = useCallback(
		(
			filePath: string,
			lineNumber: number,
			content: string,
			endLine?: number,
			severity?: CommentSeverity,
			parentId?: string,
			target?: CommentTarget,
		) => {
			const comment: LineComment = {
				id: crypto.randomUUID(),
				filePath,
				lineNumber,
				...(endLine != null && { endLine }),
				content,
				status: "unsent",
				createdAt: Date.now(),
				...(parentId != null && { parentId }),
				...(severity != null && { severity }),
				resolved: false,
				target: target ?? "local",
			};
			setComments((prev) => [...prev, comment]);
			invoke("add_comment", {
				worktreeName: worktreeNameRef.current,
				comment: lineCommentToDTO(comment),
				source: "desktop",
			}).catch(console.error);
			return comment;
		},
		[],
	);

	const removeComment = useCallback((id: string) => {
		setComments((prev) => prev.filter((c) => c.id !== id));
		invoke("remove_comment", {
			worktreeName: worktreeNameRef.current,
			id,
			source: "desktop",
		}).catch(console.error);
	}, []);

	const updateComment = useCallback((id: string, content: string) => {
		setComments((prev) =>
			prev.map((c) => (c.id === id ? { ...c, content } : c)),
		);
		invoke("update_comment_content", {
			worktreeName: worktreeNameRef.current,
			id,
			content,
			source: "desktop",
		}).catch(console.error);
	}, []);

	const markAsSent = useCallback((ids: string[]) => {
		const idSet = new Set(ids);
		setComments((prev) =>
			prev.map((c) =>
				idSet.has(c.id) ? { ...c, status: "sent" as const } : c,
			),
		);
		invoke("mark_comments_sent", {
			worktreeName: worktreeNameRef.current,
			ids,
			source: "desktop",
		}).catch(console.error);
	}, []);

	const resolveComment = useCallback((id: string) => {
		setComments((prev) =>
			prev.map((c) => (c.id === id ? { ...c, resolved: !c.resolved } : c)),
		);
		invoke("toggle_resolve_comment", {
			worktreeName: worktreeNameRef.current,
			id,
			source: "desktop",
		}).catch(console.error);
	}, []);

	const getCommentsForFile = useCallback(
		(filePath: string) => {
			return comments.filter((c) => c.filePath === filePath);
		},
		[comments],
	);

	const [showResolvedComments, setShowResolvedComments] = useState(false);

	const toggleShowResolvedComments = useCallback(() => {
		setShowResolvedComments((prev) => !prev);
	}, []);

	const unsentComments = comments.filter((c) => c.status === "unsent");

	return {
		comments,
		unsentComments,
		addComment,
		removeComment,
		updateComment,
		markAsSent,
		resolveComment,
		getCommentsForFile,
		setComments,
		showResolvedComments,
		toggleShowResolvedComments,
	};
}
