import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReviewThread, WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";
import { useRemoteReviewThreads } from "./useRemoteReviewThreads";

const makeThread = (overrides: Partial<ReviewThread> = {}): ReviewThread => ({
	id: "t1",
	worktreeName: "/repo",
	author: { kind: "human", displayName: "Human" },
	target: {},
	state: "open",
	comments: [
		{
			id: "c1",
			threadId: "t1",
			author: { kind: "human", displayName: "Human" },
			content: "Claim",
			createdAt: 1,
		},
	],
	stances: [],
	resolve: null,
	createdAt: 1,
	updatedAt: 1,
	version: 1,
	canResolve: true,
	myStance: "none",
	...overrides,
});

function setup() {
	let handler: ((msg: WsMessage) => void) | null = null;
	const subscribe: Subscribe = (next) => {
		handler = next;
		return vi.fn();
	};
	const send = vi.fn();
	const hook = renderHook(() =>
		useRemoteReviewThreads({
			subscribe,
			send,
			connected: true,
			selectedWorktree: "/repo",
		}),
	);
	return { ...hook, send, dispatch: (msg: WsMessage) => handler?.(msg) };
}

describe("useRemoteReviewThreads", () => {
	it("refreshes, upserts thread responses, formats errors, and sends operations", () => {
		const { result, send, dispatch } = setup();

		expect(send).toHaveBeenCalledWith({
			type: "review_list_request",
			payload: { worktreeName: "/repo", filter: null },
		});

		act(() => {
			dispatch({
				type: "review_list_response",
				payload: {
					success: true,
					worktreeName: "/repo",
					threads: [makeThread()],
					error: null,
				},
			});
		});
		expect(result.current.threads).toHaveLength(1);
		expect(result.current.selectedThreadId).toBe("t1");

		act(() => {
			dispatch({
				type: "review_thread_response",
				payload: {
					success: true,
					worktreeName: "/repo",
					thread: makeThread({ id: "t2", updatedAt: 2 }),
					error: null,
				},
			});
		});
		expect(result.current.threads.map((thread) => thread.id)).toEqual([
			"t2",
			"t1",
		]);

		act(() => {
			result.current.createThread("General");
			result.current.appendComment("t2", "Reply");
			result.current.setStance("t2", "agree");
			result.current.resolveThread("t2", "Done");
		});
		expect(send).toHaveBeenCalledWith({
			type: "review_create_request",
			payload: { worktreeName: "/repo", target: {}, content: "General" },
		});
		expect(send).toHaveBeenCalledWith({
			type: "review_append_comment_request",
			payload: { worktreeName: "/repo", threadId: "t2", content: "Reply" },
		});
		expect(send).toHaveBeenCalledWith({
			type: "review_set_stance_request",
			payload: { worktreeName: "/repo", threadId: "t2", value: "agree" },
		});
		expect(send).toHaveBeenCalledWith({
			type: "review_resolve_request",
			payload: {
				worktreeName: "/repo",
				threadId: "t2",
				outcome: "resolved",
				summary: "Done",
			},
		});

		act(() => {
			dispatch({
				type: "review_thread_response",
				payload: {
					success: false,
					worktreeName: "/repo",
					thread: null,
					error: { code: "already_resolved", message: "closed" },
				},
			});
		});
		expect(result.current.error).toBe("already_resolved: closed");
	});

	it("ignores stale responses from a previously selected worktree", () => {
		const { result, dispatch } = setup();

		act(() => {
			dispatch({
				type: "review_list_response",
				payload: {
					success: true,
					worktreeName: "/other",
					threads: [makeThread({ id: "stale", worktreeName: "/other" })],
					error: null,
				},
			});
		});

		expect(result.current.threads).toEqual([]);
		expect(result.current.selectedThreadId).toBeNull();
	});
});
