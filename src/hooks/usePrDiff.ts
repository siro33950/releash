import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { PrReviewComment } from "@/lib/prCommentMapping";
import { prReviewCommentsToThreads } from "@/lib/prCommentMapping";
import type { Thread } from "@/types/thread";

export interface PrFile {
	filename: string;
	status: string;
	additions: number;
	deletions: number;
}

export interface PrFileDiff {
	filename: string;
	originalContent: string;
	modifiedContent: string;
}

interface PostedComment {
	id: number;
}

interface UsePrDiffResult {
	files: PrFile[];
	loading: boolean;
	error: string | null;
	selectedFile: string | null;
	selectFile: (filename: string | null) => void;
	fileDiff: PrFileDiff | null;
	fileDiffLoading: boolean;
	reviewThreads: Thread[];
	reviewThreadsLoading: boolean;
	replyToThread: (
		threadId: string,
		body: string,
	) => Promise<PostedComment | null>;
	postPrComment: (body: string) => Promise<PostedComment | null>;
}

export function usePrDiff(
	rootPath: string,
	prNumber: number | null,
	baseRef: string | null,
	headRef: string | null,
): UsePrDiffResult {
	const [files, setFiles] = useState<PrFile[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [selectedFile, setSelectedFile] = useState<string | null>(null);
	const [fileDiff, setFileDiff] = useState<PrFileDiff | null>(null);
	const [fileDiffLoading, setFileDiffLoading] = useState(false);
	const [reviewThreads, setReviewThreads] = useState<Thread[]>([]);
	const [reviewThreadsLoading, setReviewThreadsLoading] = useState(false);

	// Fetch PR review comments when prNumber changes
	useEffect(() => {
		if (!prNumber) {
			setReviewThreads([]);
			return;
		}

		let cancelled = false;
		setReviewThreadsLoading(true);

		invoke<PrReviewComment[]>("get_pr_review_comments", {
			repoPath: rootPath,
			prNumber,
		})
			.then((comments) => {
				if (!cancelled) {
					const threads = prReviewCommentsToThreads(comments);
					setReviewThreads(threads);
					setReviewThreadsLoading(false);
				}
			})
			.catch((err) => {
				console.error("Failed to fetch PR review comments:", err);
				if (!cancelled) {
					setReviewThreads([]);
					setReviewThreadsLoading(false);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [rootPath, prNumber]);

	// Fetch file list when prNumber changes
	useEffect(() => {
		if (!prNumber) {
			setFiles([]);
			setSelectedFile(null);
			return;
		}

		let cancelled = false;
		setLoading(true);
		setError(null);
		setSelectedFile(null);

		invoke<PrFile[]>("get_pr_files", {
			repoPath: rootPath,
			prNumber,
		})
			.then((result) => {
				if (!cancelled) {
					setFiles(result);
					setLoading(false);
				}
			})
			.catch((err) => {
				if (!cancelled) {
					setError(String(err));
					setLoading(false);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [rootPath, prNumber]);

	// Fetch file content when selectedFile changes
	useEffect(() => {
		if (!selectedFile || !baseRef || !headRef) {
			setFileDiff(null);
			return;
		}

		let cancelled = false;
		setFileDiffLoading(true);
		setFileDiff(null);

		const absPath = `${rootPath}/${selectedFile}`;

		Promise.all([
			invoke<string>("get_file_at_ref", {
				filePath: absPath,
				gitRef: baseRef,
			}).catch(() => ""),
			invoke<string>("get_file_at_ref", {
				filePath: absPath,
				gitRef: headRef,
			}).catch(() => ""),
		])
			.then(([originalContent, modifiedContent]) => {
				if (!cancelled) {
					setFileDiff({
						filename: selectedFile,
						originalContent,
						modifiedContent,
					});
					setFileDiffLoading(false);
				}
			})
			.catch(() => {
				if (!cancelled) setFileDiffLoading(false);
			});

		return () => {
			cancelled = true;
		};
	}, [rootPath, selectedFile, baseRef, headRef]);

	const selectFile = useCallback((filename: string | null) => {
		setSelectedFile(filename);
	}, []);

	const replyToThread = useCallback(
		async (threadId: string, body: string): Promise<PostedComment | null> => {
			if (!prNumber) return null;
			const thread = reviewThreads.find((t) => t.id === threadId);
			if (!thread) return null;
			const firstEntry = thread.entries[0];
			if (!firstEntry?.prCommentId) return null;
			try {
				return await invoke<PostedComment>("reply_to_pr_review_comment", {
					repoPath: rootPath,
					prNumber,
					commentId: firstEntry.prCommentId,
					body,
				});
			} catch (err) {
				console.error("Failed to reply to PR review comment:", err);
				return null;
			}
		},
		[rootPath, prNumber, reviewThreads],
	);

	const postPrComment = useCallback(
		async (body: string): Promise<PostedComment | null> => {
			if (!prNumber) return null;
			try {
				return await invoke<PostedComment>("post_pr_comment", {
					repoPath: rootPath,
					prNumber,
					body,
				});
			} catch (err) {
				console.error("Failed to post PR comment:", err);
				return null;
			}
		},
		[rootPath, prNumber],
	);

	return {
		files,
		loading,
		error,
		selectedFile,
		selectFile,
		fileDiff,
		fileDiffLoading,
		reviewThreads,
		reviewThreadsLoading,
		replyToThread,
		postPrComment,
	};
}
