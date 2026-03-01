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

/** Create a pair of deferred promises for get_review_prompt + spawn_oneshot_pty */
function makeDeferredInvokePair() {
	let resolvePrompt = (_v: string) => {};
	let resolveSpawn = (_v: unknown) => {};

	mockInvoke
		.mockReturnValueOnce(
			new Promise<string>((r) => {
				resolvePrompt = r;
			}),
		)
		.mockReturnValueOnce(
			new Promise((r) => {
				resolveSpawn = r;
			}),
		);

	return { resolvePrompt, resolveSpawn };
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
		mockInvoke.mockResolvedValueOnce(null); // find_oneshot_pty on mount
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
		expect(result.current.ptyId).toBeNull();
		expect(result.current.summary).toBeNull();
		expect(result.current.output).toBe("");
	});

	it("should transition to running on successful startReview", async () => {
		mockInvoke
			.mockResolvedValueOnce("review prompt text") // get_review_prompt
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
		expect(result.current.ptyId).toBe(42);
		expect(mockInvoke).toHaveBeenCalledWith("get_review_prompt");
		expect(mockInvoke).toHaveBeenCalledWith(
			"spawn_oneshot_pty",
			expect.objectContaining({
				worktreePath: WORKTREE,
				label: "review",
			}),
		);
	});

	it("should set error status when get_review_prompt fails", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("prompt fail"));

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		expect(result.current.status).toBe("error");
	});

	it("should set error status when spawn_oneshot_pty fails", async () => {
		mockInvoke
			.mockResolvedValueOnce("review prompt text") // get_review_prompt
			.mockRejectedValueOnce(new Error("spawn fail")); // spawn_oneshot_pty

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
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should accumulate pty output", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
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

		expect(result.current.output).toBe("hello world");
	});

	it("should ignore output from other pty_ids", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
			pty_id: 10,
			session_key: "s",
			status: "running",
		});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyOutput(10, "mine ");
			emitPtyOutput(999, "not mine");
		});

		expect(result.current.output).toBe("mine ");
	});

	it("should transition to completed on status event", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
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
	});

	it("should transition to error on timeout status", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
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

	it("should transition to cancelled on cancel", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
			pty_id: 10,
			session_key: "s",
			status: "running",
		});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		act(() => {
			emitPtyStatus(10, "cancelled");
		});

		expect(result.current.status).toBe("cancelled");
	});

	it("should invoke cancel_oneshot_pty on cancelReview", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
			pty_id: 10,
			session_key: "s",
			status: "running",
		});

		const { result } = await renderReviewHook();

		await act(async () => {
			await result.current.startReview();
		});

		await act(async () => {
			result.current.cancelReview();
		});

		expect(mockInvoke).toHaveBeenCalledWith("cancel_oneshot_pty", {
			ptyId: 10,
		});
	});

	it("should reset to idle", async () => {
		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
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
		expect(result.current.ptyId).toBeNull();
		expect(result.current.output).toBe("");
	});

	it("should flush buffered output when pty_id is confirmed", async () => {
		// Use controlled promises to ensure correct ordering:
		// 1. start review
		// 2. resolve get_review_prompt → awaitingPtyRef becomes true
		// 3. emit output (buffered because pty_id not yet known)
		// 4. resolve spawn_oneshot_pty → flush buffered output
		const deferred = makeDeferredInvokePair();

		const { result } = await renderReviewHook();

		// Start review (pauses at get_review_prompt)
		act(() => {
			result.current.startReview();
		});

		// Resolve get_review_prompt → advances to awaiting spawn
		await act(async () => {
			deferred.resolvePrompt("prompt");
		});

		// Output arrives while awaiting pty_id (awaitingPtyRef is true)
		act(() => {
			emitPtyOutput(55, "buffered data");
		});

		// Resolve spawn → should flush buffered output
		await act(async () => {
			deferred.resolveSpawn({
				pty_id: 55,
				session_key: "s",
				status: "running",
			});
		});

		expect(result.current.output).toBe("buffered data");
		expect(result.current.ptyId).toBe(55);
	});

	it("should flush buffered status when pty_id is confirmed", async () => {
		const deferred = makeDeferredInvokePair();

		const { result } = await renderReviewHook();

		act(() => {
			result.current.startReview();
		});

		await act(async () => {
			deferred.resolvePrompt("prompt");
		});

		// Status arrives before pty_id is known
		act(() => {
			emitPtyStatus(77, "completed", 0);
		});

		// Resolve spawn → should flush buffered status
		await act(async () => {
			deferred.resolveSpawn({
				pty_id: 77,
				session_key: "s",
				status: "running",
			});
		});

		expect(result.current.status).toBe("completed");
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

		mockInvoke.mockResolvedValueOnce("prompt").mockResolvedValueOnce({
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

	it("should return null when reviewAgent is none", async () => {
		const { useReviewExecution } = await import("./useReviewExecution");
		const noneSettings = { ...DEFAULT_SETTINGS, reviewAgent: "none" as const };

		const { result } = renderHook(() =>
			useReviewExecution(WORKTREE, [], noneSettings),
		);
		await act(async () => {});

		mockInvoke.mockResolvedValueOnce("prompt");

		await act(async () => {
			await result.current.startReview();
		});

		// buildReviewCommand returns null for "none", so startReview bails out
		expect(result.current.status).toBe("idle");
	});

	// -----------------------------------------------------------------------
	// Mount-time state restoration from Rust
	// -----------------------------------------------------------------------

	it("should restore running review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			pty_id: 100,
			status: "running",
			started_at: 1000,
			buffered_output: "partial output",
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("running");
		expect(result.current.ptyId).toBe(100);
		expect(result.current.output).toBe("partial output");
	});

	it("should restore completed review on mount", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			pty_id: 200,
			status: "completed",
			started_at: 2000,
			buffered_output: "done output",
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("completed");
		expect(result.current.ptyId).toBe(200);
		expect(result.current.output).toBe("done output");
	});

	it("should stay idle when no review found on mount", async () => {
		// Default beforeEach already returns null for find_oneshot_pty
		const { result } = await renderReviewHook();

		expect(result.current.status).toBe("idle");
		expect(result.current.ptyId).toBeNull();
		expect(result.current.output).toBe("");
	});

	it("should receive new output after restoring", async () => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValueOnce({
			pty_id: 300,
			status: "running",
			started_at: 3000,
			buffered_output: "restored ",
		});
		mockInvoke.mockResolvedValue(undefined);

		const { result } = await renderReviewHook();

		expect(result.current.output).toBe("restored ");

		// New output arrives via pty-output event
		act(() => {
			emitPtyOutput(300, "new data");
		});

		expect(result.current.output).toBe("restored new data");
	});

	it("should prevent double start", async () => {
		let resolvePrompt = (_v: string) => {};
		mockInvoke.mockReturnValueOnce(
			new Promise<string>((r) => {
				resolvePrompt = r;
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
			resolvePrompt("prompt text");
		});

		// get_review_prompt should only be called once
		const promptCalls = mockInvoke.mock.calls.filter(
			(c) => c[0] === "get_review_prompt",
		);
		expect(promptCalls.length).toBe(1);
	});
});
