import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
	type MutableRefObject,
	type RefObject,
	useCallback,
	useEffect,
} from "react";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { trackEvent } from "@/lib/telemetry";
import type {
	CommentAuthor,
	CommentSeverity,
	CommentTarget,
	LineComment,
} from "@/types/comment";
import type { EditorAction } from "./useWorktreeGitActions";

interface UseWorktreeCommentsParams {
	comments: LineComment[];
	addComment: (
		filePath: string,
		lineNumber: number,
		content: string,
		endLine?: number,
		author?: CommentAuthor,
		severity?: CommentSeverity,
		parentId?: string,
		target?: CommentTarget,
	) => LineComment;
	removeComment: (id: string) => void;
	updateComment: (id: string, content: string) => void;
	markAsSent: (ids: string[]) => void;
	activeTabPath: string | null;
	handleOpenFile: (path: string) => Promise<void>;
	terminalRef: RefObject<TerminalTabPanelHandle | null>;
	rootPath: string;
	dispatchEditor: React.Dispatch<EditorAction>;
	commentsRef: MutableRefObject<LineComment[]>;
}

export function useWorktreeComments({
	comments,
	addComment,
	removeComment,
	updateComment,
	markAsSent,
	activeTabPath,
	handleOpenFile,
	terminalRef,
	rootPath,
	dispatchEditor,
	commentsRef,
}: UseWorktreeCommentsParams) {
	const broadcastComments = useCallback((commentsList: LineComment[]) => {
		invoke("broadcast_comments", {
			comments: {
				comments: commentsList.map((c) => ({
					id: c.id,
					file_path: c.filePath,
					line_number: c.lineNumber,
					...(c.endLine != null && { end_line: c.endLine }),
					content: c.content,
					status: c.status,
					created_at: c.createdAt,
					...(c.parentId != null && { parent_id: c.parentId }),
					author: c.author,
					...(c.severity != null && { severity: c.severity }),
					resolved: c.resolved,
					target: c.target,
				})),
			},
		}).catch(() => {});
	}, []);

	useEffect(() => {
		const unlistenComment = listen<{
			file_path: string;
			line_number: number;
			end_line?: number;
			content: string;
			author?: { type: "human" | "ai"; name: string };
			severity?: "info" | "warning" | "error" | "suggestion";
			target?: "ai" | "review" | "local";
		}>("remote-comment-added", (event) => {
			const {
				file_path,
				line_number,
				end_line,
				content,
				author,
				severity,
				target,
			} = event.payload;
			addComment(
				file_path,
				line_number,
				content,
				end_line ?? undefined,
				author,
				severity,
				undefined,
				target,
			);
		});

		const unlistenDelete = listen<{ id: string }>(
			"remote-comment-deleted",
			(event) => {
				removeComment(event.payload.id);
			},
		);

		const unlistenUpdate = listen<{ id: string; content: string }>(
			"remote-comment-updated",
			(event) => {
				updateComment(event.payload.id, event.payload.content);
			},
		);

		const unlistenConnected = listen("remote-connected", () => {
			broadcastComments(commentsRef.current);
		});

		return () => {
			unlistenComment.then((f) => f());
			unlistenDelete.then((f) => f());
			unlistenUpdate.then((f) => f());
			unlistenConnected.then((f) => f());
		};
	}, [
		addComment,
		removeComment,
		updateComment,
		broadcastComments,
		commentsRef,
	]);

	useEffect(() => {
		broadcastComments(comments);
	}, [comments, broadcastComments]);

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent, rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(text);
				terminalRef.current.writeToTerminal("\r");
				markAsSent(unsent.map((c) => c.id));
				trackEvent("comment_sent", { count: unsent.length });
			}
		},
		[markAsSent, rootPath, terminalRef],
	);

	const handleSendComment = useCallback(
		(comment: LineComment) => {
			const text = formatCommentsForTerminal([comment], rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(text);
				terminalRef.current.writeToTerminal("\r");
				markAsSent([comment.id]);
				trackEvent("comment_sent", { count: 1 });
			}
		},
		[markAsSent, rootPath, terminalRef],
	);

	const handleCopyComment = useCallback((comment: LineComment) => {
		const text = formatCommentForClipboard(comment);
		navigator.clipboard.writeText(text).catch(() => {});
		trackEvent("comment_copied");
	}, []);

	const handleCommentClick = useCallback(
		(commentFilePath: string, lineNumber: number) => {
			// MCP由来のコメントは相対パスの場合があるため、絶対パスに解決する
			const absolutePath = commentFilePath.startsWith("/")
				? commentFilePath
				: `${rootPath}/${commentFilePath}`;
			if (activeTabPath === absolutePath) {
				dispatchEditor({
					type: "SET_PENDING_REVEAL",
					reveal: { path: absolutePath, line: lineNumber },
				});
			} else {
				handleOpenFile(absolutePath);
				dispatchEditor({
					type: "SET_PENDING_REVEAL",
					reveal: { path: absolutePath, line: lineNumber },
				});
			}
		},
		[activeTabPath, handleOpenFile, dispatchEditor, rootPath],
	);

	return {
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		handleCommentClick,
	};
}
