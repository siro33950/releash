import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "@/types/settings";
import type { Thread } from "@/types/thread";

// ---------------------------------------------------------------------------
// Mocks
// ---------------------------------------------------------------------------

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

type ListenCallback = (event: { payload: Record<string, unknown> }) => void;
let capturedListeners: Map<string, ListenCallback>;

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn((eventName: string, callback: ListenCallback) => {
		capturedListeners.set(eventName, callback);
		return Promise.resolve(() => {
			capturedListeners.delete(eventName);
		});
	}),
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const WORKTREE = "/tmp/test-worktree";
const SETTINGS = { ...DEFAULT_SETTINGS, reviewAgent: "claude" as const };

function makeThread(overrides: Partial<Thread> = {}): Thread {
	return {
		id: "t-1",
		filePath: "src/file.ts",
		lineNumber: 10,
		entries: [
			{
				id: "e-1",
				content: "review issue",
				isAi: true,
				createdAt: Date.now(),
			},
		],
		createdAt: Date.now(),
		resolved: false,
		severity: "warning",
		...overrides,
	};
}

/** Emit a review-state-changed event via the captured listener */
function emitReviewStateChanged(payload: {
	status: string;
	file_states: { file_path: string; status: string; pty_id: number | null }[];
	progress: { done: number; total: number; error_count: number };
}) {
	const listener = capturedListeners.get("review-state-changed");
	listener?.({ payload: payload as unknown as Record<string, unknown> });
}

/** Emit a review-file-output event via the captured listener */
function emitReviewFileOutput(file_path: string, data: string) {
	const listener = capturedListeners.get("review-file-output");
	listener?.({ payload: { file_path, data } });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("useReviewExecution", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		capturedListeners = new Map();
		// get_review_status on mount (returns null = no active session)
		mockInvoke.mockResolvedValueOnce(null);
		mockInvoke.mockResolvedValue(undefined);
	});

	async function renderReviewHook(
		worktreePath: string | null = WORKTREE,
		threads: Thread[] = [],
	) {
		const { useReviewExecution } = await import("./useReviewExecution");
		const utils = renderHook(
			(props: { wt: string | null; threads: Thread[] }) =>
				useReviewExecution(props.wt, props.threads, SETTINGS),
			{ initialProps: { wt: worktreePath, threads } },
		);
		// flush useEffect listeners
		await act(async () => {});
		return utils;
	}

	it("should start in idle status", async () => {
		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("idle");
		expect(result.current.summary).toBeNull();
		expect(result.current.fileStates).toEqual([]);
	});

	it("should call start_review on startReview", async () => {
		// start_review returns session id
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"start_review",
			expect.objectContaining({
				worktreePath: WORKTREE,
				commandTemplate: expect.any(String),
				concurrency: 5,
			}),
		);
	});

	it("should transition to running when Rust emits running state", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 42 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		expect(result.current.status).toBe("running");
		expect(result.current.fileStates.length).toBe(1);
		expect(result.current.fileStates[0].filePath).toBe("src/a.ts");
	});

	it("should set error status when start_review invoke fails", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("start fail"));

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("error");
	});

	it("should not start when worktreePath is null", async () => {
		const { result } = await renderReviewHook(null);

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("idle");
		// start_review should not have been called
		const startCalls = mockInvoke.mock.calls.filter(
			(c) => c[0] === "start_review",
		);
		expect(startCalls).toHaveLength(0);
	});

	it("should accumulate file output per file", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		// Rust emits running state with file
		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		act(() => {
			emitReviewFileOutput("src/a.ts", "hello ");
		});
		act(() => {
			emitReviewFileOutput("src/a.ts", "world");
		});

		const fileState = result.current.fileStates.find(
			(f) => f.filePath === "src/a.ts",
		);
		expect(fileState?.output).toBe("hello world");
	});

	it("should transition to completed via event", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		act(() => {
			emitReviewStateChanged({
				status: "completed",
				file_states: [{ file_path: "src/a.ts", status: "done", pty_id: 10 }],
				progress: { done: 1, total: 1, error_count: 0 },
			});
		});

		expect(result.current.status).toBe("completed");
		expect(result.current.progress).toEqual({ done: 1, total: 1 });
	});

	it("should transition to error via event when error_count > 0", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "error",
				file_states: [{ file_path: "src/a.ts", status: "error", pty_id: 10 }],
				progress: { done: 1, total: 1, error_count: 1 },
			});
		});

		expect(result.current.status).toBe("error");
	});

	it("should call cancel_review on cancelReview", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		await act(async () => {
			await result.current.cancelReview();
		});

		expect(mockInvoke).toHaveBeenCalledWith("cancel_review", {
			reviewSessionId: WORKTREE,
		});
	});

	it("should transition to cancelled via event after cancel", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		await act(async () => {
			await result.current.cancelReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "cancelled",
				file_states: [{ file_path: "src/a.ts", status: "error", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		expect(result.current.status).toBe("cancelled");
	});

	it("should reset to idle", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		expect(result.current.status).toBe("running");

		act(() => {
			result.current.reset();
		});

		expect(result.current.status).toBe("idle");
		expect(result.current.fileStates).toEqual([]);
		expect(mockInvoke).toHaveBeenCalledWith("reset_review", {
			reviewSessionId: WORKTREE,
		});
	});

	it("should track progress across multiple files", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [
					{ file_path: "src/a.ts", status: "running", pty_id: 10 },
					{ file_path: "src/b.ts", status: "pending", pty_id: null },
				],
				progress: { done: 0, total: 2, error_count: 0 },
			});
		});

		expect(result.current.progress).toEqual({ done: 0, total: 2 });

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [
					{ file_path: "src/a.ts", status: "done", pty_id: 10 },
					{ file_path: "src/b.ts", status: "running", pty_id: 11 },
				],
				progress: { done: 1, total: 2, error_count: 0 },
			});
		});

		expect(result.current.progress?.done).toBe(1);
		expect(result.current.status).toBe("running");

		act(() => {
			emitReviewStateChanged({
				status: "completed",
				file_states: [
					{ file_path: "src/a.ts", status: "done", pty_id: 10 },
					{ file_path: "src/b.ts", status: "done", pty_id: 11 },
				],
				progress: { done: 2, total: 2, error_count: 0 },
			});
		});

		expect(result.current.progress).toEqual({ done: 2, total: 2 });
		expect(result.current.status).toBe("completed");
	});

	it("should complete with error if any file fails", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "error",
				file_states: [
					{ file_path: "src/a.ts", status: "done", pty_id: 10 },
					{ file_path: "src/b.ts", status: "error", pty_id: 11 },
				],
				progress: { done: 2, total: 2, error_count: 1 },
			});
		});

		expect(result.current.status).toBe("error");
		expect(result.current.progress).toEqual({ done: 2, total: 2 });

		const aState = result.current.fileStates.find(
			(f) => f.filePath === "src/a.ts",
		);
		const bState = result.current.fileStates.find(
			(f) => f.filePath === "src/b.ts",
		);
		expect(aState?.status).toBe("done");
		expect(bState?.status).toBe("error");
	});

	it("should handle no-tasks as completed with empty summary", async () => {
		// start_review returns null (no tasks to review)
		mockInvoke.mockResolvedValueOnce(null);

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("completed");
		expect(result.current.summary?.total).toBe(0);
	});

	it("should compute summary when status is completed", async () => {
		const now = Date.now();
		const threads: Thread[] = [
			makeThread({ id: "t-e1", severity: "error", createdAt: now + 100 }),
			makeThread({ id: "t-w1", severity: "warning", createdAt: now + 200 }),
			makeThread({ id: "t-i1", severity: "info", createdAt: now + 300 }),
			makeThread({
				id: "t-s1",
				severity: "suggestion",
				createdAt: now + 400,
			}),
			// Old thread - should not be counted
			makeThread({ id: "t-old", severity: "error", createdAt: now - 100000 }),
			// Non-review thread (isAi: false) - should not be counted
			makeThread({
				id: "t-local",
				severity: "error",
				entries: [
					{
						id: "e-local",
						content: "local issue",
						isAi: false,
						createdAt: now + 500,
					},
				],
				createdAt: now + 500,
			}),
		];

		mockInvoke.mockResolvedValueOnce("session-1");

		const { result, rerender } = await renderReviewHook(WORKTREE, []);

		await act(async () => {
			await result.current.startReview();
		});

		// Provide threads and complete via event
		rerender({ wt: WORKTREE, threads });

		act(() => {
			emitReviewStateChanged({
				status: "completed",
				file_states: [{ file_path: "src/a.ts", status: "done", pty_id: 10 }],
				progress: { done: 1, total: 1, error_count: 0 },
			});
		});

		// Wait for summary computation useEffect
		await act(async () => {});

		expect(result.current.summary).not.toBeNull();
		expect(result.current.summary?.total).toBe(4);
		expect(result.current.summary?.errors).toBe(1);
		expect(result.current.summary?.warnings).toBe(1);
		expect(result.current.summary?.infos).toBe(1);
		expect(result.current.summary?.suggestions).toBe(1);
	});

	it("should return error when reviewAgent is none", async () => {
		const { useReviewExecution } = await import("./useReviewExecution");
		const noneSettings = { ...DEFAULT_SETTINGS, reviewAgent: "none" as const };

		const { result } = renderHook(() =>
			useReviewExecution(WORKTREE, [], noneSettings),
		);
		await act(async () => {});

		await act(async () => {
			await result.current.startReview();
		});

		// buildReviewCommandTemplate returns null for "none" → error
		expect(result.current.status).toBe("error");
		// start_review should not have been called
		const startCalls = mockInvoke.mock.calls.filter(
			(c) => c[0] === "start_review",
		);
		expect(startCalls).toHaveLength(0);
	});

	// -----------------------------------------------------------------------
	// Mount-time state restoration from Rust
	// -----------------------------------------------------------------------

	it("should restore running review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			status: "running",
			file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 100 }],
			progress: { done: 0, total: 1, error_count: 0 },
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("running");
		expect(result.current.fileStates.length).toBe(1);
		expect(result.current.fileStates[0].filePath).toBe("src/a.ts");
		expect(result.current.fileStates[0].status).toBe("running");
	});

	it("should restore completed review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			status: "completed",
			file_states: [{ file_path: "src/b.ts", status: "done", pty_id: 200 }],
			progress: { done: 1, total: 1, error_count: 0 },
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("completed");
		expect(result.current.fileStates[0].status).toBe("done");
	});

	it("should stay idle when no review found on mount", async () => {
		// Default beforeEach already returns null for get_review_status
		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("idle");
		expect(result.current.fileStates).toEqual([]);
	});

	it("should stay idle when get_review_status returns idle status", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			status: "idle",
			file_states: [],
			progress: { done: 0, total: 0, error_count: 0 },
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("idle");
	});

	it("should not register listeners when worktreePath is null", async () => {
		await renderReviewHook(null);

		// No listeners should be registered for review events
		expect(capturedListeners.has("review-state-changed")).toBe(false);
		expect(capturedListeners.has("review-file-output")).toBe(false);
	});

	it("should not update state after unmount", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result, unmount } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 60 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		unmount();

		// Emitting events after unmount should not throw
		expect(() => {
			emitReviewStateChanged({
				status: "completed",
				file_states: [{ file_path: "src/a.ts", status: "done", pty_id: 60 }],
				progress: { done: 1, total: 1, error_count: 0 },
			});
		}).not.toThrow();
	});

	it("should preserve output on state change events", async () => {
		mockInvoke.mockResolvedValueOnce("session-1");

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		// Rust emits running state
		act(() => {
			emitReviewStateChanged({
				status: "running",
				file_states: [{ file_path: "src/a.ts", status: "running", pty_id: 10 }],
				progress: { done: 0, total: 1, error_count: 0 },
			});
		});

		// Output arrives
		act(() => {
			emitReviewFileOutput("src/a.ts", "some output");
		});

		expect(result.current.fileStates[0].output).toBe("some output");

		// State changes again (e.g., file completes) — output should be preserved
		act(() => {
			emitReviewStateChanged({
				status: "completed",
				file_states: [{ file_path: "src/a.ts", status: "done", pty_id: 10 }],
				progress: { done: 1, total: 1, error_count: 0 },
			});
		});

		expect(result.current.fileStates[0].output).toBe("some output");
	});
});
