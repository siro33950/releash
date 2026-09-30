import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StateTarget } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import type { ReviewDiscussionThread } from "@/types/diffComment";
import { useDiffComments } from "./useDiffComments";

const states = stateSubscriptions();
const mockInvoke = vi.fn();

vi.mock("@/lib/client", () => ({
	subscribeState: (...args: unknown[]) =>
		(states.subscribeState as (...args: unknown[]) => () => void)(...args),
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
}));

const target: StateTarget<"review-threads"> = {
	kind: "review-threads",
	args: ["wt"],
};

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
		states.clear();
	});

	it("worktree名でreview threadを購読し届いた一覧を公開する", () => {
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
		expect(states.subscribeState).toHaveBeenCalledWith(
			target,
			expect.any(Function),
			expect.any(Function),
		);
		expect(result.current.loading).toBe(true);

		const comments = [makeThread()];
		act(() => states.publish(target, comments));

		expect(result.current.comments).toEqual(comments);
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("購読の失敗を保持し次の配信で解除する", () => {
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
		act(() => states.publish(target, [makeThread()]));
		act(() => states.fail(target, { message: "read denied" }));
		expect(result.current.error).toBe("read denied");
		expect(result.current.comments).toHaveLength(1);

		act(() => states.publish(target, []));
		expect(result.current.error).toBeNull();
		expect(result.current.comments).toEqual([]);
	});

	it("worktree名が空なら購読せず空の一覧を返す", () => {
		const { result } = renderHook(() => useDiffComments({ worktreeName: "" }));
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
		expect(result.current.comments).toEqual([]);
	});

	it("worktree名が変わると新しい対象を購読する", () => {
		const { result, rerender } = renderHook(
			({ name }) => useDiffComments({ worktreeName: name }),
			{ initialProps: { name: "wt" } },
		);
		act(() => states.publish(target, [makeThread()]));
		rerender({ name: "other" });
		expect(result.current.comments).toEqual([]);
		expect(states.subscribeState).toHaveBeenLastCalledWith(
			{ kind: "review-threads", args: ["other"] },
			expect.any(Function),
			expect.any(Function),
		);
	});

	it("creates a review thread for a new diff comment", async () => {
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
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
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
		mockInvoke.mockResolvedValue(makeThread({ id: "new" }));

		await act(async () => {
			await result.current.addComment({ content: "General note" });
		});

		expect(mockInvoke).toHaveBeenCalledWith("create_review_thread", {
			worktreeName: "wt",
			filePath: null,
			lineNumber: null,
			endLine: null,
			content: "General note",
		});
	});

	it("calls review mutation commands", async () => {
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
		mockInvoke.mockResolvedValue(null);

		await act(async () => {
			await result.current.appendComment("t1", "Reply");
			await result.current.resolveThread("t1", "resolved", "Done");
			await result.current.deleteThread("t1");
		});

		expect(mockInvoke).toHaveBeenCalledWith("append_review_comment", {
			worktreeName: "wt",
			threadId: "t1",
			content: "Reply",
		});
		expect(mockInvoke).toHaveBeenCalledWith("resolve_review_thread", {
			worktreeName: "wt",
			threadId: "t1",
			outcome: "resolved",
			summary: "Done",
		});
		expect(mockInvoke).toHaveBeenCalledWith("delete_review_thread", {
			worktreeName: "wt",
			threadId: "t1",
		});
	});

	it.each([
		"addComment",
		"appendComment",
		"resolveThread",
		"deleteThread",
	] as const)("%sの失敗を通知して呼び出し元へ返す", async (operation) => {
		const notice = vi.fn();
		window.addEventListener("releash-client-error", notice);
		try {
			const error = new Error(`${operation} failed`);
			mockInvoke.mockRejectedValueOnce(error);
			const { result } = renderHook(() =>
				useDiffComments({ worktreeName: "wt" }),
			);
			const calls = {
				addComment: () => result.current.addComment({ content: "note" }),
				appendComment: () => result.current.appendComment("t1", "reply"),
				resolveThread: () =>
					result.current.resolveThread("t1", "resolved", "done"),
				deleteThread: () => result.current.deleteThread("t1"),
			};
			await expect(calls[operation]()).rejects.toBe(error);
			expect(notice).toHaveBeenCalledOnce();
			expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
				`${operation} failed`,
			);
		} finally {
			window.removeEventListener("releash-client-error", notice);
		}
	});

	it("getCommentsForFile filters by thread target filePath", () => {
		const { result } = renderHook(() =>
			useDiffComments({ worktreeName: "wt" }),
		);
		act(() =>
			states.publish(target, [
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
			]),
		);

		expect(result.current.getCommentsForFile("a.ts")).toHaveLength(2);
	});
});
