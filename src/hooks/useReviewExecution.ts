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

export interface ReviewExecutionState {
	status: ReviewStatus;
	ptyId: number | null;
	summary: ReviewSummary | null;
	output: string;
}

export function useReviewExecution(
	worktreePath: string | null,
	comments: LineComment[],
	settings: AppSettings,
) {
	const [state, setState] = useState<ReviewExecutionState>({
		status: "idle",
		ptyId: null,
		summary: null,
		output: "",
	});

	const reviewStartTimeRef = useRef(0);
	const ptyIdRef = useRef<number | null>(null);
	const awaitingPtyRef = useRef(false);
	const prevStatusRef = useRef<ReviewStatus>("idle");
	const pendingStatusRef = useRef<
		Map<number, { status: string; exit_code: number | null }>
	>(new Map());
	const pendingOutputRef = useRef<Map<number, string>>(new Map());

	const applyStatusChange = useCallback((ptyId: number, status: string) => {
		setState((prev) => {
			if (prev.ptyId !== ptyId) return prev;

			if (
				status === "completed" ||
				status === "error" ||
				status === "timeout"
			) {
				return {
					...prev,
					status: status === "completed" ? "completed" : "error",
				};
			}
			if (status === "cancelled") {
				return { ...prev, status: "cancelled" };
			}
			if (status === "running") {
				return { ...prev, status: "running" };
			}
			return prev;
		});
	}, []);

	// Listen for oneshot PTY status changes (mount once)
	useEffect(() => {
		const unlisten = listen<{
			pty_id: number;
			status: string;
			exit_code: number | null;
		}>("oneshot-pty-status-changed", (event) => {
			const { pty_id, status, exit_code } = event.payload;

			if (ptyIdRef.current === pty_id) {
				applyStatusChange(pty_id, status);
			} else {
				pendingStatusRef.current.set(pty_id, { status, exit_code });
			}
		});

		return () => {
			unlisten.then((f) => f());
		};
	}, [applyStatusChange]);

	// Capture PTY output — buffer by pty_id until confirmed (mount once)
	useEffect(() => {
		const unlisten = listen<{ pty_id: number; data: string }>(
			"pty-output",
			(event) => {
				const { pty_id, data } = event.payload;

				if (ptyIdRef.current === pty_id) {
					setState((prev) => ({
						...prev,
						output: prev.output + data,
					}));
				} else if (awaitingPtyRef.current) {
					const buf = pendingOutputRef.current.get(pty_id) ?? "";
					pendingOutputRef.current.set(pty_id, buf + data);
				}
			},
		);

		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	// Mount-time: read current review state from Rust (the sole source of truth)
	useEffect(() => {
		if (!worktreePath) return;

		const token = runTokenRef.current;

		invoke<{
			pty_id: number;
			status: string;
			started_at: number;
			buffered_output: string;
		} | null>("find_oneshot_pty", {
			worktreePath,
			label: "review",
		})
			.then((result) => {
				if (!result || runTokenRef.current !== token) return;

				const statusMap: Record<string, ReviewStatus> = {
					starting: "running",
					running: "running",
					completed: "completed",
					error: "error",
					timeout: "error",
					cancelled: "cancelled",
				};
				const mapped = statusMap[result.status];
				if (!mapped) return;

				ptyIdRef.current = result.pty_id;
				reviewStartTimeRef.current = result.started_at * 1000;

				setState({
					status: mapped,
					ptyId: result.pty_id,
					summary: null,
					output: result.buffered_output,
				});
			})
			.catch(() => {});
	}, [worktreePath]);

	// Compute summary when status is completed (re-computes on new comments)
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

	const startInFlightRef = useRef(false);
	const runTokenRef = useRef(0);

	const startReview = useCallback(async () => {
		if (!worktreePath) return;
		if (startInFlightRef.current) return;
		startInFlightRef.current = true;
		const runToken = ++runTokenRef.current;

		let prompt: string;
		try {
			prompt = await invoke<string>("get_review_prompt");
		} catch {
			startInFlightRef.current = false;
			setState({ status: "error", ptyId: null, summary: null, output: "" });
			return;
		}
		if (runTokenRef.current !== runToken) return;

		const command = buildReviewCommand(settings, prompt);
		if (!command) {
			startInFlightRef.current = false;
			return;
		}

		reviewStartTimeRef.current = Date.now();
		ptyIdRef.current = null;
		awaitingPtyRef.current = true;
		pendingStatusRef.current.clear();
		pendingOutputRef.current.clear();

		setState({ status: "starting", ptyId: null, summary: null, output: "" });

		try {
			const info = await invoke<{
				pty_id: number;
				session_key: string;
				status: string;
			}>("spawn_oneshot_pty", {
				command,
				worktreePath,
				label: "review",
				timeoutSecs: null,
			});
			if (runTokenRef.current !== runToken) return;

			ptyIdRef.current = info.pty_id;
			awaitingPtyRef.current = false;

			// Flush buffered output for this pty_id
			const buffered = pendingOutputRef.current.get(info.pty_id) ?? "";
			pendingOutputRef.current.clear();
			if (buffered) {
				setState((prev) => ({
					...prev,
					status: "running",
					ptyId: info.pty_id,
					output: prev.output + buffered,
				}));
			} else {
				setState((prev) => ({
					...prev,
					status: "running",
					ptyId: info.pty_id,
				}));
			}

			// Flush buffered status if process already finished
			const pendingStatus = pendingStatusRef.current.get(info.pty_id);
			pendingStatusRef.current.delete(info.pty_id);
			if (pendingStatus) {
				applyStatusChange(info.pty_id, pendingStatus.status);
			}
		} catch {
			ptyIdRef.current = null;
			awaitingPtyRef.current = false;
			setState({ status: "error", ptyId: null, summary: null, output: "" });
		} finally {
			startInFlightRef.current = false;
		}
	}, [worktreePath, settings, applyStatusChange]);

	const cancelReview = useCallback(async () => {
		const id = ptyIdRef.current;
		if (id == null) return;
		try {
			await invoke("cancel_oneshot_pty", { ptyId: id });
		} catch {
			// ignore
		}
	}, []);

	const reset = useCallback(() => {
		runTokenRef.current += 1;
		ptyIdRef.current = null;
		awaitingPtyRef.current = false;
		startInFlightRef.current = false;
		pendingStatusRef.current.clear();
		pendingOutputRef.current.clear();
		setState({ status: "idle", ptyId: null, summary: null, output: "" });
	}, []);

	return {
		...state,
		startReview,
		cancelReview,
		reset,
	};
}
