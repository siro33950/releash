import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { type AppSettings, buildReviewCommandTemplate } from "@/types/settings";
import type { Thread } from "@/types/thread";
import { getThreadOrigin } from "@/types/thread";

export interface ReviewSummary {
	total: number;
	errors: number;
	warnings: number;
	infos: number;
	suggestions: number;
}

export type ReviewStatus =
	| "idle"
	| "running"
	| "completed"
	| "error"
	| "cancelled";

export type FileReviewStatus = "pending" | "running" | "done" | "error";

export interface FileReviewState {
	filePath: string;
	status: FileReviewStatus;
	ptyId: number | null;
	output: string;
}

interface ReviewStateChanged {
	status: ReviewStatus;
	file_states: {
		file_path: string;
		status: FileReviewStatus;
		pty_id: number | null;
	}[];
	progress: {
		done: number;
		total: number;
		error_count: number;
	};
}

interface ReviewFileOutput {
	file_path: string;
	data: string;
}

export interface ReviewExecutionState {
	status: ReviewStatus;
	progress: { done: number; total: number } | null;
	summary: ReviewSummary | null;
	fileStates: FileReviewState[];
}

export function useReviewExecution(
	worktreePath: string | null,
	threads: Thread[],
	settings: AppSettings,
) {
	const [state, setState] = useState<ReviewExecutionState>({
		status: "idle",
		progress: null,
		summary: null,
		fileStates: [],
	});

	const reviewStartTimeRef = useRef(0);
	const settingsRef = useRef(settings);
	settingsRef.current = settings;
	const fileOutputRef = useRef<Map<string, string>>(new Map());

	// Listen for review state changes from Rust orchestrator
	useEffect(() => {
		if (!worktreePath) return;

		const unlisten = listen<ReviewStateChanged>(
			"review-state-changed",
			(event) => {
				const { status, file_states, progress } = event.payload;
				setState((prev) => ({
					...prev,
					status,
					progress: { done: progress.done, total: progress.total },
					fileStates: file_states.map((f) => ({
						filePath: f.file_path,
						status: f.status,
						ptyId: f.pty_id,
						output: fileOutputRef.current.get(f.file_path) ?? "",
					})),
				}));
			},
		);

		return () => {
			unlisten.then((f) => f());
		};
	}, [worktreePath]);

	// Listen for per-file output from Rust orchestrator
	useEffect(() => {
		if (!worktreePath) return;

		const unlisten = listen<ReviewFileOutput>("review-file-output", (event) => {
			const { file_path, data } = event.payload;
			const current = fileOutputRef.current.get(file_path) ?? "";
			fileOutputRef.current.set(file_path, current + data);

			setState((prev) => ({
				...prev,
				fileStates: prev.fileStates.map((f) =>
					f.filePath === file_path
						? {
								...f,
								output: fileOutputRef.current.get(file_path) ?? "",
							}
						: f,
				),
			}));
		});

		return () => {
			unlisten.then((f) => f());
		};
	}, [worktreePath]);

	// Mount-time recovery: check if Rust has an active review session
	useEffect(() => {
		if (!worktreePath) return;
		let active = true;

		invoke<ReviewStateChanged | null>("get_review_status", {
			reviewSessionId: worktreePath,
		})
			.then((status) => {
				if (!active || !status) return;
				if (status.status === "idle") return;
				reviewStartTimeRef.current = Date.now();
				setState({
					status: status.status,
					progress: {
						done: status.progress.done,
						total: status.progress.total,
					},
					summary: null,
					fileStates: status.file_states.map((f) => ({
						filePath: f.file_path,
						status: f.status,
						ptyId: f.pty_id,
						output: "",
					})),
				});
			})
			.catch(() => {});

		return () => {
			active = false;
		};
	}, [worktreePath]);

	// Compute summary when status is completed
	useEffect(() => {
		if (state.status !== "completed") return;

		const reviewThreads = threads.filter(
			(t) =>
				getThreadOrigin(t) === "ai-review" &&
				t.createdAt > reviewStartTimeRef.current,
		);
		const summary: ReviewSummary = {
			total: reviewThreads.length,
			errors: reviewThreads.filter((t) => t.severity === "error").length,
			warnings: reviewThreads.filter((t) => t.severity === "warning").length,
			infos: reviewThreads.filter((t) => t.severity === "info").length,
			suggestions: reviewThreads.filter((t) => t.severity === "suggestion")
				.length,
		};
		setState((prev) => ({ ...prev, summary }));
	}, [state.status, threads]);

	const startReview = useCallback(async () => {
		if (!worktreePath) return;

		const commandTemplate = buildReviewCommandTemplate(settingsRef.current);
		if (!commandTemplate) {
			setState({
				status: "error",
				progress: null,
				summary: null,
				fileStates: [],
			});
			return;
		}

		reviewStartTimeRef.current = Date.now();
		fileOutputRef.current.clear();

		try {
			const sessionId = await invoke<string | null>("start_review", {
				worktreePath,
				commandTemplate,
				concurrency: Math.max(1, settingsRef.current.reviewConcurrency ?? 5),
			});

			if (!sessionId) {
				setState({
					status: "completed",
					progress: { done: 0, total: 0 },
					summary: {
						total: 0,
						errors: 0,
						warnings: 0,
						infos: 0,
						suggestions: 0,
					},
					fileStates: [],
				});
			}
		} catch {
			setState({
				status: "error",
				progress: null,
				summary: null,
				fileStates: [],
			});
		}
	}, [worktreePath]);

	const cancelReview = useCallback(async () => {
		if (!worktreePath) return;
		await invoke("cancel_review", {
			reviewSessionId: worktreePath,
		}).catch(() => {});
	}, [worktreePath]);

	const reset = useCallback(() => {
		if (worktreePath) {
			invoke("reset_review", { reviewSessionId: worktreePath }).catch(() => {});
		}
		fileOutputRef.current.clear();
		setState({
			status: "idle",
			progress: null,
			summary: null,
			fileStates: [],
		});
	}, [worktreePath]);

	return {
		...state,
		startReview,
		cancelReview,
		reset,
	};
}
