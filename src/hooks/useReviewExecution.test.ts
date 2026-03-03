import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LineComment } from "@/types/comment";
import { DEFAULT_SETTINGS } from "@/types/settings";

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

function makeComment(overrides: Partial<LineComment> = {}): LineComment {
	return {
		id: "c-1",
		filePath: "src/file.ts",
		lineNumber: 10,
		content: "review issue",
		status: "unsent",
		createdAt: Date.now(),
		resolved: false,
		target: "review",
		severity: "warning",
		...overrides,
	};
}

function makeTask(filePath: string) {
	return { file_path: filePath, prompt: `Review ${filePath}` };
}

/** Emit a pty-output event via the captured listener */
function emitPtyOutput(pty_id: number, data: string) {
	const listener = capturedListeners.get("pty-output");
	listener?.({ payload: { pty_id, data } });
}

/** Emit a oneshot-pty-status-changed event via the captured listener */
function emitPtyStatus(
	pty_id: number,
	status: string,
	exit_code: number | null = null,
) {
	const listener = capturedListeners.get("oneshot-pty-status-changed");
	listener?.({ payload: { pty_id, status, exit_code } });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("useReviewExecution", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		capturedListeners = new Map();
		// list_oneshot_ptys on mount (returns empty array = no recovery)
		mockInvoke.mockResolvedValueOnce([]);
		mockInvoke.mockResolvedValue(undefined);
	});

	async function renderReviewHook(
		worktreePath: string | null = WORKTREE,
		comments: LineComment[] = [],
	) {
		const { useReviewExecution } = await import("./useReviewExecution");
		const utils = renderHook(
			(props: { wt: string | null; comments: LineComment[] }) =>
				useReviewExecution(props.wt, props.comments, SETTINGS),
			{ initialProps: { wt: worktreePath, comments } },
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

	it("should transition to running on successful startReview", async () => {
		const tasks = [makeTask("src/a.ts")];
		mockInvoke
			.mockResolvedValueOnce(tasks) // get_per_file_review_tasks
			.mockResolvedValueOnce({
				// spawn_oneshot_pty
				pty_id: 42,
				session_key: "sess-1",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("running");
		expect(result.current.fileStates.length).toBe(1);
		expect(result.current.fileStates[0].filePath).toBe("src/a.ts");
		expect(mockInvoke).toHaveBeenCalledWith("get_per_file_review_tasks", {
			worktreePath: WORKTREE,
		});
		expect(mockInvoke).toHaveBeenCalledWith(
			"spawn_oneshot_pty",
			expect.objectContaining({
				worktreePath: WORKTREE,
				label: "review:src/a.ts",
			}),
		);
	});

	it("should set error status when get_per_file_review_tasks fails", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("task fail"));

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("error");
	});

	it("should set error status when spawn_oneshot_pty fails", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")]) // get_per_file_review_tasks
			.mockRejectedValueOnce(new Error("spawn fail")); // spawn_oneshot_pty

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		// spawnNextTask is async, wait for it to complete
		await act(async () => {});

		// Single file failed → all done → error
		expect(result.current.status).toBe("error");
	});

	it("should not start when worktreePath is null", async () => {
		const { result } = await renderReviewHook(null);

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("idle");
		// Only the mount-time list_oneshot_ptys (which didn't fire for null path)
	});

	it("should accumulate pty output per file", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyOutput(10, "hello ");
		});
		act(() => {
			emitPtyOutput(10, "world");
		});

		const fileState = result.current.fileStates.find(
			(f) => f.filePath === "src/a.ts",
		);
		expect(fileState?.output).toBe("hello world");
	});

	it("should transition to completed when all files done", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyStatus(10, "completed", 0);
		});

		expect(result.current.status).toBe("completed");
		expect(result.current.progress).toEqual({ done: 1, total: 1 });
	});

	it("should transition to error on timeout status", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyStatus(10, "timeout");
		});

		expect(result.current.status).toBe("error");
	});

	it("should cancel all running PTYs on cancelReview", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		await act(async () => {
			await result.current.cancelReview();
		});

		expect(mockInvoke).toHaveBeenCalledWith("cancel_oneshot_pty", {
			ptyId: 10,
		});
		expect(result.current.status).toBe("cancelled");
	});

	it("should reset to idle", async () => {
		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("running");

		act(() => {
			result.current.reset();
		});

		expect(result.current.status).toBe("idle");
		expect(result.current.fileStates).toEqual([]);
	});

	it("should track progress across multiple files", async () => {
		const tasks = [makeTask("src/a.ts"), makeTask("src/b.ts")];
		mockInvoke
			.mockResolvedValueOnce(tasks)
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s1",
				status: "running",
			})
			.mockResolvedValueOnce({
				pty_id: 11,
				session_key: "s2",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.progress).toEqual({ done: 0, total: 2 });

		act(() => {
			emitPtyStatus(10, "completed", 0);
		});

		expect(result.current.progress?.done).toBe(1);
		expect(result.current.status).toBe("running");

		act(() => {
			emitPtyStatus(11, "completed", 0);
		});

		expect(result.current.progress).toEqual({ done: 2, total: 2 });
		expect(result.current.status).toBe("completed");
	});

	it("should complete with error if any file fails", async () => {
		const tasks = [makeTask("src/a.ts"), makeTask("src/b.ts")];
		mockInvoke
			.mockResolvedValueOnce(tasks)
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s1",
				status: "running",
			})
			.mockResolvedValueOnce({
				pty_id: 11,
				session_key: "s2",
				status: "running",
			});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyStatus(10, "completed", 0);
		});
		act(() => {
			emitPtyStatus(11, "error");
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

	it("should handle empty task list as completed", async () => {
		mockInvoke.mockResolvedValueOnce([]); // no changed files

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("completed");
		expect(result.current.summary?.total).toBe(0);
	});

	it("should compute summary when status is completed", async () => {
		const now = Date.now();
		const comments: LineComment[] = [
			makeComment({ id: "e1", severity: "error", createdAt: now + 100 }),
			makeComment({ id: "w1", severity: "warning", createdAt: now + 200 }),
			makeComment({ id: "i1", severity: "info", createdAt: now + 300 }),
			makeComment({ id: "s1", severity: "suggestion", createdAt: now + 400 }),
			// Old comment - should not be counted
			makeComment({ id: "old", severity: "error", createdAt: now - 100000 }),
			// Non-review comment - should not be counted
			makeComment({
				id: "local",
				severity: "error",
				target: "local",
				createdAt: now + 500,
			}),
		];

		mockInvoke
			.mockResolvedValueOnce([makeTask("src/a.ts")])
			.mockResolvedValueOnce({
				pty_id: 10,
				session_key: "s",
				status: "running",
			});

		const { result, rerender } = await renderReviewHook(WORKTREE, []);

		await act(async () => {
			await result.current.startReview();
		});

		// Provide comments and complete
		rerender({ wt: WORKTREE, comments });

		act(() => {
			emitPtyStatus(10, "completed", 0);
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

		// get_per_file_review_tasks returns tasks but buildReviewCommand returns null
		mockInvoke.mockResolvedValueOnce([makeTask("src/a.ts")]);

		await act(async () => {
			await result.current.startReview();
		});

		// spawnNextTask is async (uses ref), wait for it to complete
		await act(async () => {});

		// All tasks fail (buildReviewCommand returns null for "none") → error
		expect(result.current.status).toBe("error");
	});

	// -----------------------------------------------------------------------
	// Mount-time state restoration from Rust
	// -----------------------------------------------------------------------

	it("should restore running review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce([
			{
				pty_id: 100,
				label: "review:src/a.ts",
				status: "running",
				started_at: 1000,
				buffered_output: "partial output",
			},
		]);
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("running");
		expect(result.current.fileStates.length).toBe(1);
		expect(result.current.fileStates[0].filePath).toBe("src/a.ts");
		expect(result.current.fileStates[0].output).toBe("partial output");
	});

	it("should restore completed review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce([
			{
				pty_id: 200,
				label: "review:src/b.ts",
				status: "completed",
				started_at: 2000,
				buffered_output: "done output",
			},
		]);
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("completed");
		expect(result.current.fileStates[0].status).toBe("done");
		expect(result.current.fileStates[0].output).toBe("done output");
	});

	it("should stay idle when no review found on mount", async () => {
		// Default beforeEach already returns [] for list_oneshot_ptys
		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("idle");
		expect(result.current.fileStates).toEqual([]);
	});

	it("should prevent double start", async () => {
		let resolveTasks = (_v: unknown) => {};
		mockInvoke.mockReturnValueOnce(
			new Promise((r) => {
				resolveTasks = r;
			}),
		);

		const { result } = await renderReviewHook();

		// Start first review (still pending)
		act(() => {
			result.current.startReview();
		});

		// Try to start second review while first is pending
		act(() => {
			result.current.startReview();
		});

		// Resolve the first
		await act(async () => {
			resolveTasks([]);
		});

		// get_per_file_review_tasks should only be called once
		const taskCalls = mockInvoke.mock.calls.filter(
			(c) => c[0] === "get_per_file_review_tasks",
		);
		expect(taskCalls.length).toBe(1);
	});

	it("should spawn next task from queue when one completes (concurrency control)", async () => {
		const concurrencySettings = { ...SETTINGS, reviewConcurrency: 1 };
		const { useReviewExecution } = await import("./useReviewExecution");

		// Reset mock for this specific test
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce([]); // list_oneshot_ptys on mount

		const tasks = [makeTask("src/a.ts"), makeTask("src/b.ts")];

		mockInvoke
			.mockResolvedValueOnce(tasks) // get_per_file_review_tasks
			.mockResolvedValueOnce({
				// spawn first
				pty_id: 10,
				session_key: "s1",
				status: "running",
			});

		const { result } = renderHook(() =>
			useReviewExecution(WORKTREE, [], concurrencySettings),
		);
		await act(async () => {});

		await act(async () => {
			await result.current.startReview();
		});

		// Only first file should be running (concurrency = 1)
		expect(result.current.fileStates[0].status).toBe("running");
		expect(result.current.fileStates[1].status).toBe("pending");

		// Complete first → should spawn second
		mockInvoke.mockResolvedValueOnce({
			pty_id: 11,
			session_key: "s2",
			status: "running",
		});

		await act(async () => {
			emitPtyStatus(10, "completed", 0);
		});

		// Wait for spawn
		await act(async () => {});

		expect(result.current.fileStates[0].status).toBe("done");
		expect(
			mockInvoke.mock.calls.filter((c) => c[0] === "spawn_oneshot_pty").length,
		).toBe(2);
	});
});
