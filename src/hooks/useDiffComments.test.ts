import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDiffComments } from "./useDiffComments";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeComment = (overrides: Record<string, unknown> = {}) => ({
	id: "c1",
	filePath: "src/main.ts",
	lineNumber: 10,
	endLine: null,
	content: "Fix this",
	status: "unsent",
	createdAt: Date.now(),
	...overrides,
});

describe("useDiffComments", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("loads comments on mount", async () => {
		const comments = [makeComment()];
		mockInvoke.mockResolvedValue(comments);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "my-worktree" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toEqual(comments);
		});

		expect(mockInvoke).toHaveBeenCalledWith("load_diff_comments", {
			worktreeName: "my-worktree",
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

	it("computes unsentCount from comments", async () => {
		const comments = [
			makeComment({ id: "c1", status: "unsent" }),
			makeComment({ id: "c2", status: "sent" }),
			makeComment({ id: "c3", status: "unsent" }),
		];
		mockInvoke.mockResolvedValue(comments);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.unsentCount).toBe(2);
		});
	});

	it("addComment calls invoke with correct params", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		const newComment = makeComment({ id: "new" });
		mockInvoke.mockResolvedValue(newComment);

		await act(async () => {
			await result.current.addComment({
				filePath: "src/main.ts",
				lineNumber: 5,
				content: "New comment",
			});
		});

		expect(mockInvoke).toHaveBeenCalledWith("add_diff_comment", {
			worktreeName: "wt",
			filePath: "src/main.ts",
			lineNumber: 5,
			endLine: null,
			content: "New comment",
		});
	});

	it("updateComment calls invoke with correct params", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		mockInvoke.mockResolvedValue(undefined);

		await act(async () => {
			await result.current.updateComment("c1", "Updated content");
		});

		expect(mockInvoke).toHaveBeenCalledWith("update_diff_comment", {
			worktreeName: "wt",
			commentId: "c1",
			content: "Updated content",
		});
	});

	it("deleteComment calls invoke with correct params", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		mockInvoke.mockResolvedValue(undefined);

		await act(async () => {
			await result.current.deleteComment("c1");
		});

		expect(mockInvoke).toHaveBeenCalledWith("delete_diff_comment", {
			worktreeName: "wt",
			commentId: "c1",
		});
	});

	it("sendToAgent calls invoke with comment IDs", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		mockInvoke.mockResolvedValue({
			sentCount: 2,
			formattedMessage: "msg",
		});

		await act(async () => {
			await result.current.sendToAgent(["c1", "c2"]);
		});

		expect(mockInvoke).toHaveBeenCalledWith("send_diff_comments_to_agent", {
			worktreeName: "wt",
			commentIds: ["c1", "c2"],
		});
	});

	it("sendToAgent returns mentions from invoke result", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		const mentions = [
			{ filePath: "src/main.ts", startLine: 10, endLine: null },
			{ filePath: "src/app.tsx", startLine: 5, endLine: 15 },
		];
		mockInvoke.mockResolvedValue({
			sentCount: 2,
			formattedMessage: "msg",
			mentions,
			commentIds: ["c1", "c2"],
		});

		let sendResult:
			| Awaited<ReturnType<typeof result.current.sendToAgent>>
			| undefined;
		await act(async () => {
			sendResult = await result.current.sendToAgent(["c1", "c2"]);
		});

		expect(sendResult?.mentions).toEqual(mentions);
		expect(sendResult?.commentIds).toEqual(["c1", "c2"]);
	});

	it("sendAllUnsent calls sendToAgent with empty array", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		mockInvoke.mockResolvedValue({
			sentCount: 1,
			formattedMessage: "msg",
		});

		await act(async () => {
			await result.current.sendAllUnsent();
		});

		expect(mockInvoke).toHaveBeenCalledWith("send_diff_comments_to_agent", {
			worktreeName: "wt",
			commentIds: [],
		});
	});

	it("getCommentsForFile filters by filePath", async () => {
		const comments = [
			makeComment({ id: "c1", filePath: "a.ts" }),
			makeComment({ id: "c2", filePath: "b.ts" }),
			makeComment({ id: "c3", filePath: "a.ts" }),
		];
		mockInvoke.mockResolvedValue(comments);

		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);

		await waitFor(() => {
			expect(result.current.comments).toHaveLength(3);
		});

		const filtered = result.current.getCommentsForFile("a.ts");
		expect(filtered).toHaveLength(2);
		expect(filtered.every((c) => c.filePath === "a.ts")).toBe(true);
	});

	it("subscribes to diff-comments-changed event", async () => {
		mockInvoke.mockResolvedValue([]);

		renderHook(() => useDiffComments({ worktreeName: "wt" }));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledWith(
				"diff-comments-changed",
				expect.any(Function),
			);
		});
	});
});
