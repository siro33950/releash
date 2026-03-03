import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import type { LineComment } from "@/types/comment";
import { type AppSettings, buildReviewCommand } from "@/types/settings";

export interface ReviewSummary {
	total: number;
	errors: number;
	warnings: number;
	infos: number;
	suggestions: number;
}

export type ReviewStatus =
	| "idle"
	| "starting"
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

interface PerFileReviewTask {
	file_path: string;
	prompt: string;
}

export interface ReviewExecutionState {
	status: ReviewStatus;
	ptyIds: Set<number>;
	progress: { done: number; total: number } | null;
	summary: ReviewSummary | null;
	fileStates: FileReviewState[];
}

export function useReviewExecution(
	worktreePath: string | null,
	comments: LineComment[],
	settings: AppSettings,
) {
	const [state, setState] = useState<ReviewExecutionState>({
		status: "idle",
		ptyIds: new Set(),
		progress: null,
		summary: null,
		fileStates: [],
	});

	const reviewStartTimeRef = useRef(0);
	const prevStatusRef = useRef<ReviewStatus>("idle");

	// Mutable refs for concurrency control
	const ptyIdSetRef = useRef<Set<number>>(new Set());
	const fileStatesRef = useRef<FileReviewState[]>([]);
	const taskQueueRef = useRef<PerFileReviewTask[]>([]);
	const activeCountRef = useRef(0);
	const doneCountRef = useRef(0);
	const totalCountRef = useRef(0);
	const errorCountRef = useRef(0);
	const runTokenRef = useRef(0);
	const startInFlightRef = useRef(false);
	const concurrencyRef = useRef(5);

	// Per-file output buffers (for PTY output arriving before ptyId is confirmed)
	const pendingOutputRef = useRef<Map<number, string>>(new Map());
	const pendingStatusRef = useRef<
		Map<number, { status: string; exit_code: number | null }>
	>(new Map());
	// Map pty_id → runToken so events use the correct token (not the current one)
	const ptyRunTokenMapRef = useRef<Map<number, number>>(new Map());

	// Stable refs for settings to avoid circular useCallback dependencies
	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const settingsRef = useRef(settings);
	settingsRef.current = settings;

	const syncState = useCallback(() => {
		const allDone =
			totalCountRef.current > 0 &&
			doneCountRef.current >= totalCountRef.current;
		setState((prev) => ({
			...prev,
			...(allDone
				? { status: errorCountRef.current > 0 ? "error" : "completed" }
				: {}),
			ptyIds: new Set(ptyIdSetRef.current),
			progress: {
				done: doneCountRef.current,
				total: totalCountRef.current,
			},
			fileStates: [...fileStatesRef.current],
		}));
	}, []);

	// Use refs to break circular dependency between spawnNextTask and handlePtyFinished
	const spawnNextTaskRef = useRef<(runToken: number) => void>(() => {});
	const handlePtyFinishedRef = useRef<
		(ptyId: number, status: string, runToken: number) => void
	>(() => {});

	const spawnNextTask = useCallback(
		async (runToken: number) => {
			const wt = worktreePathRef.current;
			if (!wt) return;
			if (runTokenRef.current !== runToken) return;

			const task = taskQueueRef.current.shift();
			if (!task) return;

			activeCountRef.current++;

			const fileIdx = fileStatesRef.current.findIndex(
				(f) => f.filePath === task.file_path,
			);

			const command = buildReviewCommand(settingsRef.current, task.prompt);
			if (!command) {
				if (fileIdx >= 0) {
					fileStatesRef.current[fileIdx] = {
						...fileStatesRef.current[fileIdx],
						status: "error",
					};
				}
				activeCountRef.current--;
				doneCountRef.current++;
				errorCountRef.current++;
				syncState();
				spawnNextTaskRef.current(runToken);
				return;
			}

			try {
				const info = await invoke<{
					pty_id: number;
					session_key: string;
					status: string;
				}>("spawn_oneshot_pty", {
					command,
					worktreePath: wt,
					label: `review:${task.file_path}`,
					timeoutSecs: null,
				});

				if (runTokenRef.current !== runToken) {
					activeCountRef.current = Math.max(0, activeCountRef.current - 1);
					// Token changed — cancel the orphaned PTY
					invoke("cancel_oneshot_pty", { ptyId: info.pty_id }).catch(() => {});
					return;
				}

				ptyIdSetRef.current.add(info.pty_id);
				ptyRunTokenMapRef.current.set(info.pty_id, runToken);

				if (fileIdx >= 0) {
					fileStatesRef.current[fileIdx] = {
						...fileStatesRef.current[fileIdx],
						status: "running",
						ptyId: info.pty_id,
					};
				}

				// Flush pending output
				const buffered = pendingOutputRef.current.get(info.pty_id);
				if (buffered && fileIdx >= 0) {
					fileStatesRef.current[fileIdx] = {
						...fileStatesRef.current[fileIdx],
						output: fileStatesRef.current[fileIdx].output + buffered,
					};
				}
				pendingOutputRef.current.delete(info.pty_id);

				// Flush pending status if process already finished
				const pendingStatus = pendingStatusRef.current.get(info.pty_id);
				if (pendingStatus) {
					pendingStatusRef.current.delete(info.pty_id);
					handlePtyFinishedRef.current(
						info.pty_id,
						pendingStatus.status,
						runToken,
					);
				}

				syncState();
			} catch {
				if (fileIdx >= 0) {
					fileStatesRef.current[fileIdx] = {
						...fileStatesRef.current[fileIdx],
						status: "error",
					};
				}
				activeCountRef.current--;
				doneCountRef.current++;
				errorCountRef.current++;
				syncState();
				spawnNextTaskRef.current(runToken);
			}
		},
		[syncState],
	);

	const handlePtyFinished = useCallback(
		(ptyId: number, status: string, runToken: number) => {
			if (runTokenRef.current !== runToken) return;

			ptyIdSetRef.current.delete(ptyId);
			ptyRunTokenMapRef.current.delete(ptyId);
			pendingStatusRef.current.delete(ptyId);
			pendingOutputRef.current.delete(ptyId);

			const fileIdx = fileStatesRef.current.findIndex((f) => f.ptyId === ptyId);
			if (fileIdx < 0) return;

			const isError =
				status === "error" || status === "timeout" || status === "cancelled";
			fileStatesRef.current[fileIdx] = {
				...fileStatesRef.current[fileIdx],
				status: isError ? "error" : "done",
			};

			activeCountRef.current--;
			doneCountRef.current++;
			if (isError) errorCountRef.current++;

			syncState();
			if (doneCountRef.current < totalCountRef.current) {
				spawnNextTaskRef.current(runToken);
			}
		},
		[syncState],
	);

	// Keep refs in sync
	spawnNextTaskRef.current = spawnNextTask;
	handlePtyFinishedRef.current = handlePtyFinished;

	// Listen for oneshot PTY status changes
	useEffect(() => {
		const unlisten = listen<{
			pty_id: number;
			status: string;
			exit_code: number | null;
		}>("oneshot-pty-status-changed", (event) => {
			const { pty_id, status } = event.payload;

			const isTerminal =
				status === "completed" ||
				status === "error" ||
				status === "timeout" ||
				status === "cancelled";

			if (!isTerminal) return;

			// Check if this pty_id belongs to our review session
			if (ptyIdSetRef.current.has(pty_id)) {
				const token =
					ptyRunTokenMapRef.current.get(pty_id) ?? runTokenRef.current;
				ptyRunTokenMapRef.current.delete(pty_id);
				handlePtyFinishedRef.current(pty_id, status, token);
			} else if (activeCountRef.current > 0 || startInFlightRef.current) {
				// Buffer it - might arrive before spawn returns
				pendingStatusRef.current.set(pty_id, {
					status,
					exit_code: event.payload.exit_code,
				});
			}
		});

		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	// Capture PTY output per-file
	useEffect(() => {
		const unlisten = listen<{ pty_id: number; data: string }>(
			"pty-output",
			(event) => {
				const { pty_id, data } = event.payload;

				const fileIdx = fileStatesRef.current.findIndex(
					(f) => f.ptyId === pty_id,
				);
				if (fileIdx >= 0) {
					fileStatesRef.current[fileIdx] = {
						...fileStatesRef.current[fileIdx],
						output: fileStatesRef.current[fileIdx].output + data,
					};
					setState((prev) => ({
						...prev,
						fileStates: [...fileStatesRef.current],
					}));
				} else if (activeCountRef.current > 0 || startInFlightRef.current) {
					// Buffer output for PTY IDs we haven't matched yet
					const buf = pendingOutputRef.current.get(pty_id) ?? "";
					pendingOutputRef.current.set(pty_id, buf + data);
				}
			},
		);

		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	// Mount-time recovery: find running review PTYs
	useEffect(() => {
		if (!worktreePath) return;

		const token = runTokenRef.current;
		let active = true;

		invoke<
			{
				pty_id: number;
				label: string;
				status: string;
				started_at: number;
				buffered_output: string;
			}[]
		>("list_oneshot_ptys", { worktreePath })
			.then((sessions) => {
				if (!active || runTokenRef.current !== token) return;

				const reviewSessions = sessions.filter((s) =>
					s.label.startsWith("review:"),
				);
				if (reviewSessions.length === 0) return;

				const minStartedAt = Math.min(
					...reviewSessions.map((s) => s.started_at),
				);
				reviewStartTimeRef.current = minStartedAt * 1000;

				const fileStates: FileReviewState[] = reviewSessions.map((s) => {
					const filePath = s.label.replace("review:", "");
					const statusMap: Record<string, FileReviewStatus> = {
						starting: "running",
						running: "running",
						completed: "done",
						error: "error",
						timeout: "error",
						cancelled: "error",
					};
					return {
						filePath,
						status: statusMap[s.status] ?? "pending",
						ptyId: s.pty_id,
						output: s.buffered_output,
					};
				});

				const ptyIds = new Set(reviewSessions.map((s) => s.pty_id));
				const doneCount = fileStates.filter(
					(f) => f.status === "done" || f.status === "error",
				).length;
				const hasRunning = fileStates.some((f) => f.status === "running");

				ptyIdSetRef.current = ptyIds;
				fileStatesRef.current = fileStates;
				totalCountRef.current = fileStates.length;
				doneCountRef.current = doneCount;
				activeCountRef.current = fileStates.filter(
					(f) => f.status === "running",
				).length;

				const hasError = fileStates.some((f) => f.status === "error");
				const overallStatus: ReviewStatus = hasRunning
					? "running"
					: doneCount >= fileStates.length
						? hasError
							? "error"
							: "completed"
						: "running";

				setState({
					status: overallStatus,
					ptyIds,
					progress: { done: doneCount, total: fileStates.length },
					summary: null,
					fileStates,
				});
			})
			.catch(() => {});

		return () => {
			active = false;
		};
	}, [worktreePath]);

	// Compute summary when status is completed
	useEffect(() => {
		if (state.status === "completed") {
			const reviewComments = comments.filter(
				(c) =>
					c.target === "review" && c.createdAt > reviewStartTimeRef.current,
			);
			const summary: ReviewSummary = {
				total: reviewComments.length,
				errors: reviewComments.filter((c) => c.severity === "error").length,
				warnings: reviewComments.filter((c) => c.severity === "warning").length,
				infos: reviewComments.filter((c) => c.severity === "info").length,
				suggestions: reviewComments.filter((c) => c.severity === "suggestion")
					.length,
			};
			setState((prev) => ({ ...prev, summary }));
		}
		prevStatusRef.current = state.status;
	}, [state.status, comments]);

	const startReview = useCallback(async () => {
		if (!worktreePath) return;
		if (startInFlightRef.current) return;
		startInFlightRef.current = true;
		const runToken = ++runTokenRef.current;

		// Get per-file tasks from Rust
		let tasks: PerFileReviewTask[];
		try {
			tasks = await invoke<PerFileReviewTask[]>("get_per_file_review_tasks", {
				worktreePath,
			});
		} catch {
			startInFlightRef.current = false;
			setState({
				status: "error",
				ptyIds: new Set(),
				progress: null,
				summary: null,
				fileStates: [],
			});
			return;
		}

		if (runTokenRef.current !== runToken) {
			startInFlightRef.current = false;
			return;
		}

		if (tasks.length === 0) {
			startInFlightRef.current = false;
			setState({
				status: "completed",
				ptyIds: new Set(),
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
			return;
		}

		// Initialize state
		const concurrency = Math.max(1, settings.reviewConcurrency ?? 5);
		concurrencyRef.current = concurrency;
		reviewStartTimeRef.current = Date.now();
		ptyIdSetRef.current = new Set();
		ptyRunTokenMapRef.current.clear();
		totalCountRef.current = tasks.length;
		doneCountRef.current = 0;
		activeCountRef.current = 0;
		errorCountRef.current = 0;
		pendingStatusRef.current.clear();
		pendingOutputRef.current.clear();

		const fileStates: FileReviewState[] = tasks.map((t) => ({
			filePath: t.file_path,
			status: "pending" as FileReviewStatus,
			ptyId: null,
			output: "",
		}));
		fileStatesRef.current = fileStates;
		taskQueueRef.current = [...tasks];

		setState({
			status: "running",
			ptyIds: new Set(),
			progress: { done: 0, total: tasks.length },
			summary: null,
			fileStates: [...fileStates],
		});

		// Spawn initial batch
		const initialBatch = Math.min(concurrency, tasks.length);
		for (let i = 0; i < initialBatch; i++) {
			spawnNextTaskRef.current(runToken);
		}
		startInFlightRef.current = false;
	}, [worktreePath, settings]);

	const cancelReview = useCallback(async () => {
		// Invalidate current run token to prevent completion events from overwriting status
		runTokenRef.current += 1;
		// Cancel all running PTYs
		const cancelPromises = Array.from(ptyIdSetRef.current).map((id) =>
			invoke("cancel_oneshot_pty", { ptyId: id }).catch(() => {}),
		);
		// Clear the queue
		taskQueueRef.current = [];
		await Promise.all(cancelPromises);
		ptyIdSetRef.current.clear();
		ptyRunTokenMapRef.current.clear();
		activeCountRef.current = 0;
		pendingStatusRef.current.clear();
		pendingOutputRef.current.clear();
		setState((prev) => ({
			...prev,
			status: "cancelled",
			ptyIds: new Set(),
		}));
	}, []);

	const reset = useCallback(() => {
		runTokenRef.current += 1;
		ptyIdSetRef.current = new Set();
		ptyRunTokenMapRef.current.clear();
		fileStatesRef.current = [];
		taskQueueRef.current = [];
		activeCountRef.current = 0;
		doneCountRef.current = 0;
		totalCountRef.current = 0;
		errorCountRef.current = 0;
		startInFlightRef.current = false;
		pendingStatusRef.current.clear();
		pendingOutputRef.current.clear();
		setState({
			status: "idle",
			ptyIds: new Set(),
			progress: null,
			summary: null,
			fileStates: [],
		});
	}, []);

	return {
		...state,
		startReview,
		cancelReview,
		reset,
	};
}
