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

	// Compute summary once when status transitions to completed
	useEffect(() => {
		if (state.status === "completed" && prevStatusRef.current !== "completed") {
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

		let prompt: string;
		try {
			prompt = await invoke<string>("get_review_prompt");
		} catch {
			setState({ status: "error", ptyId: null, summary: null, output: "" });
			return;
		}

		const command = buildReviewCommand(settings, prompt);
		if (!command) return;

		reviewStartTimeRef.current = Date.now();
		ptyIdRef.current = null;
		awaitingPtyRef.current = true;
		pendingStatusRef.current.clear();

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
				timeoutSecs: 300,
			});

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
		ptyIdRef.current = null;
		setState({ status: "idle", ptyId: null, summary: null, output: "" });
	}, []);

	return {
		...state,
		startReview,
		cancelReview,
		reset,
	};
}
