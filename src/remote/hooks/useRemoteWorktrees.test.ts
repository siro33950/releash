import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WorktreeEntryMsg, WsMessage } from "@/types/protocol";
import { useRemoteWorktrees } from "./useRemoteWorktrees";

const makeWorktree = (
	overrides: Partial<WorktreeEntryMsg> = {},
): WorktreeEntryMsg => ({
	name: "wt",
	path: "/repo/wt",
	branch: "main",
	is_main: false,
	is_locked: false,
	dirty_count: 0,
	base_branch: null,
	...overrides,
});

const setupHook = (connected = false) => {
	let handler: ((msg: WsMessage) => void) | null = null;
	const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
		handler = cb;
		return vi.fn();
	});
	const send = vi.fn();
	const view = renderHook(() =>
		useRemoteWorktrees({ subscribe, send, connected }),
	);
	return { view, send, subscribe, emit: (msg: WsMessage) => handler?.(msg) };
};

describe("useRemoteWorktrees", () => {
	it("connected 時に worktree_list_request を送る", () => {
		const { send } = setupHook(true);
		expect(send).toHaveBeenCalledWith({
			type: "worktree_list_request",
			payload: {},
		});
	});

	it("worktree_list_response で一覧を反映する（PR 無し）", () => {
		const { view, emit } = setupHook(true);
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: { worktrees: [makeWorktree({ path: "/repo/a" })] },
			});
		});
		expect(view.result.current.worktrees).toHaveLength(1);
		expect(view.result.current.worktrees[0].has_pr).toBeUndefined();
		expect(view.result.current.loading).toBe(false);
	});

	it("worktree_pr_status_sync で PR をマージする", () => {
		const { view, emit } = setupHook(true);
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: { worktrees: [makeWorktree({ path: "/repo/a" })] },
			});
		});
		act(() => {
			emit({
				type: "worktree_pr_status_sync",
				payload: {
					entries: [{ path: "/repo/a", pr_number: 42, pr_url: "http://x/42" }],
				},
			});
		});
		expect(view.result.current.worktrees[0]).toMatchObject({
			path: "/repo/a",
			has_pr: true,
			pr_number: 42,
			pr_url: "http://x/42",
		});
	});

	it("PR 反映後に同じ path を含む worktree_list_response が来ても PR を保持する（ちらつき防止）", () => {
		const { view, emit } = setupHook(true);
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: { worktrees: [makeWorktree({ path: "/repo/a" })] },
			});
		});
		act(() => {
			emit({
				type: "worktree_pr_status_sync",
				payload: {
					entries: [{ path: "/repo/a", pr_number: 42, pr_url: "http://x/42" }],
				},
			});
		});
		// 後追い sync が届く前に再度一覧だけが返ってきた状況を再現する。
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: {
					worktrees: [
						makeWorktree({ path: "/repo/a" }),
						makeWorktree({ path: "/repo/b" }),
					],
				},
			});
		});
		const a = view.result.current.worktrees.find((w) => w.path === "/repo/a");
		expect(a?.has_pr).toBe(true);
		expect(a?.pr_number).toBe(42);
	});

	it("worktree_list_response で消えた worktree の PR は除去する", () => {
		const { view, emit } = setupHook(true);
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: { worktrees: [makeWorktree({ path: "/repo/a" })] },
			});
		});
		act(() => {
			emit({
				type: "worktree_pr_status_sync",
				payload: {
					entries: [{ path: "/repo/a", pr_number: 42, pr_url: "http://x/42" }],
				},
			});
		});
		// /repo/a が一覧から消える（worktree 削除）。
		act(() => {
			emit({
				type: "worktree_list_response",
				payload: { worktrees: [makeWorktree({ path: "/repo/b" })] },
			});
		});
		expect(view.result.current.worktrees).toHaveLength(1);
		expect(view.result.current.worktrees[0].path).toBe("/repo/b");
		expect(view.result.current.worktrees[0].has_pr).toBeUndefined();
	});

	it("unmount で unsubscribe する", () => {
		const unsubscribe = vi.fn();
		const subscribe = vi.fn(() => unsubscribe);
		const { unmount } = renderHook(() =>
			useRemoteWorktrees({ subscribe, send: vi.fn(), connected: false }),
		);
		unmount();
		expect(unsubscribe).toHaveBeenCalled();
	});
});
