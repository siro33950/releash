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
	stances: [],
	resolve: null,
	createdAt: Date.now(),
	updatedAt: Date.now(),
	version: 1,
	canResolve: true,
	myStance: "none",
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
			await result.current.setStance("t1", "agree");
			await result.current.resolveThread("t1", "resolved", "Done");
		});

		expect(mockInvoke).toHaveBeenCalledWith("append_review_comment", {
			worktreeName: "wt",
			threadId: "t1",
			content: "Reply",
		});
		expect(mockInvoke).toHaveBeenCalledWith("set_review_stance", {
			worktreeName: "wt",
			threadId: "t1",
			value: "agree",
		});
		expect(mockInvoke).toHaveBeenCalledWith("resolve_review_thread", {
			worktreeName: "wt",
			threadId: "t1",
			outcome: "resolved",
			summary: "Done",
		});
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
