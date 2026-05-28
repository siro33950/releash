import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import { useDiffComments } from "./useDiffComments";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeThread = (
	overrides: Partial<ReviewDiscussionThread> = {},
): ReviewDiscussionThread => ({
	id: "t1",
	worktreeName: "wt",
	author: { kind: "human", displayName: "Human" },
	target: { filePath: "src/main.ts", lineNumber: 10, endLine: null },
	state: "open",
	comments: [
		{
			id: "c1",
			threadId: "t1",
			author: { kind: "human", displayName: "Human" },
			content: "Fix this",
			createdAt: Date.now(),
		},
	],
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	...overrides,
});

describe("useDiffComments", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("loads review threads on mount", async () => {
		const comments = [makeThread()];
		mockInvoke.mockResolvedValue(comments);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "my-worktree" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toEqual(comments);
		});

		expect(mockInvoke).toHaveBeenCalledWith("list_review_threads", {
			worktreeName: "my-worktree",
			filter: null,
		});
	});

	it("returns empty array when worktreeName is empty", async () => {
		const { result } = renderHook(() => useDiffComments({ worktreeName: "" }));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.comments).toEqual([]);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("creates a review thread for a new diff comment", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		mockInvoke.mockResolvedValue(makeThread({ id: "new" }));

		await act(async () => {
			await result.current.addComment({
				filePath: "src/main.ts",
				lineNumber: 5,
				content: "New comment",
			});
		});

		expect(mockInvoke).toHaveBeenCalledWith("create_review_thread", {
			worktreeName: "wt",
			filePath: "src/main.ts",
			lineNumber: 5,
			endLine: null,
			content: "New comment",
		});
	});

	it("creates a position-independent review thread", async () => {
		mockInvoke.mockResolvedValue([]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.addComment({ content: "General claim" });
		});

		expect(mockInvoke).toHaveBeenCalledWith("create_review_thread", {
			worktreeName: "wt",
			filePath: null,
			lineNumber: null,
			endLine: null,
			content: "General claim",
		});
	});

	it("calls review mutation commands", async () => {
		mockInvoke.mockResolvedValue([]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.appendComment("t1", "Reply");
			await result.current.appendComment("t1", "Another reply");
			await result.current.resolveThread("t1", "resolved", "Done");
		});

		expect(mockInvoke).toHaveBeenCalledWith("append_review_comment", {
			worktreeName: "wt",
			threadId: "t1",
			content: "Reply",
		});
		expect(mockInvoke).toHaveBeenCalledWith("append_review_comment", {
			worktreeName: "wt",
			threadId: "t1",
			content: "Another reply",
		});
		expect(mockInvoke).toHaveBeenCalledWith("resolve_review_thread", {
			worktreeName: "wt",
			threadId: "t1",
			outcome: "resolved",
			summary: "Done",
		});
	});

	it("invokes delete_review_thread with the worktree name and thread id", async () => {
		mockInvoke.mockResolvedValue([]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.deleteThread("t1");
		});

		expect(mockInvoke).toHaveBeenCalledWith("delete_review_thread", {
			worktreeName: "wt",
			threadId: "t1",
		});
	});

	it("reloads comments when the review-comments-changed event matches the worktree", async () => {
		let listenerHandler: ((event: { payload: string }) => void) | undefined;
		mockListen.mockImplementation((_eventName, handler) => {
			listenerHandler = handler as (event: { payload: string }) => void;
			return Promise.resolve(() => {});
		});

		mockInvoke.mockResolvedValue([makeThread()]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(1);
		});

		mockInvoke.mockResolvedValue([]);

		await act(async () => {
			listenerHandler?.({ payload: "wt" });
		});

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(0);
		});
	});

	it("reloads comments when the review-comments-changed event payload is wildcard '*'", async () => {
		let listenerHandler: ((event: { payload: string }) => void) | undefined;
		mockListen.mockImplementation((_eventName, handler) => {
			listenerHandler = handler as (event: { payload: string }) => void;
			return Promise.resolve(() => {});
		});

		mockInvoke.mockResolvedValue([makeThread()]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(1);
		});

		mockInvoke.mockResolvedValue([
			makeThread({ id: "t1" }),
			makeThread({ id: "t2" }),
		]);

		await act(async () => {
			listenerHandler?.({ payload: "*" });
		});

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(2);
		});
	});

	it("ignores review-comments-changed events for other worktrees", async () => {
		let listenerHandler: ((event: { payload: string }) => void) | undefined;
		mockListen.mockImplementation((_eventName, handler) => {
			listenerHandler = handler as (event: { payload: string }) => void;
			return Promise.resolve(() => {});
		});

		mockInvoke.mockResolvedValue([makeThread()]);
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(1);
		});

		const callCountBefore = mockInvoke.mock.calls.length;

		await act(async () => {
			listenerHandler?.({ payload: "other-worktree" });
		});

		expect(mockInvoke.mock.calls.length).toBe(callCountBefore);
	});

	it("getCommentsForFile filters by thread target filePath", async () => {
		const comments = [
			makeThread({
				id: "t1",
				target: { filePath: "a.ts", lineNumber: 1, endLine: null },
			}),
			makeThread({
				id: "t2",
				target: { filePath: "b.ts", lineNumber: 1, endLine: null },
			}),
			makeThread({
				id: "t3",
				target: { filePath: "a.ts", lineNumber: 2, endLine: null },
			}),
		];
		mockInvoke.mockResolvedValue(comments);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(3);
		});

		expect(result.current.getCommentsForFile("a.ts")).toHaveLength(2);
	});

	it("subscribes to review-comments-changed event", async () => {
		mockInvoke.mockResolvedValue([]);

		renderHook(() => useDiffComments({ worktreeName: "wt" }));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"review-comments-changed",
				expect.any(Function),
			);
		});
	});
});
