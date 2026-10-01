import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { StateTarget } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import type { ReviewSnapshot } from "@/types/review";
import { useReviewSnapshot } from "./useReviewSnapshot";

const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	subscribeState: (...args: unknown[]) =>
		(states.subscribeState as (...args: unknown[]) => () => void)(...args),
}));

function snapshot(overrides: Partial<ReviewSnapshot> = {}): ReviewSnapshot {
	return {
		version: 4,
		stale: false,
		loading: false,
		base: "head",
		files: [],
		stagedFiles: [],
		changedFiles: [],
		diffStats: [],
		tree: [],
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
		...overrides,
	};
}

describe("useReviewSnapshot", () => {
	beforeEach(() => states.clear());

	it("worktreeとbaseで購読し届いた一覧をそのまま公開する", () => {
		const { result } = renderHook(() => useReviewSnapshot("/repo", "head"));
		expect(states.subscribeState).toHaveBeenCalledWith(
			{ kind: "review-snapshot", args: ["/repo", "head"] },
			expect.any(Function),
			expect.any(Function),
		);
		expect(result.current.loading).toBe(true);

		act(() =>
			states.publish(
				{ kind: "review-snapshot", args: ["/repo", "head"] },
				snapshot({
					stagedFiles: [
						{ path: "staged.ts", index_status: "new", worktree_status: "none" },
					],
					changedFiles: [
						{
							path: "changed.ts",
							index_status: "none",
							worktree_status: "modified",
						},
					],
					stagedFileCount: 1,
					changesFileCount: 1,
				}),
			),
		);

		expect(result.current.loading).toBe(false);
		expect(result.current.version).toBe(4);
		expect(result.current.stagedFiles.map((file) => file.path)).toEqual([
			"staged.ts",
		]);
		expect(result.current.changedFiles.map((file) => file.path)).toEqual([
			"changed.ts",
		]);
		expect(result.current.stagedFileCount).toBe(1);
		expect(result.current.changesFileCount).toBe(1);
	});

	it("Repository未選択では購読せず空の一覧を返す", () => {
		const { result } = renderHook(() => useReviewSnapshot(null, "branch-base"));
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
		expect(result.current.snapshot.base).toBe("branch-base");
		expect(result.current.files).toEqual([]);
	});

	it("購読の失敗では空の一覧を表示し読み込み中にしない", () => {
		const { result } = renderHook(() => useReviewSnapshot("/repo", "head"));
		act(() =>
			states.fail(
				{ kind: "review-snapshot", args: ["/repo", "head"] },
				new Error("denied"),
			),
		);
		expect(result.current.loading).toBe(false);
		expect(result.current.stagedFiles).toEqual([]);
	});

	it("読み直しの失敗では前の差分を消し回復後の差分を表示する", () => {
		const target = {
			kind: "review-snapshot" as const,
			args: ["/repo", "head"],
		};
		const { result } = renderHook(() => useReviewSnapshot("/repo", "head"));
		act(() =>
			states.publish(target, snapshot({ version: 7, changesFileCount: 3 })),
		);
		act(() => states.fail(target, new Error("scan failed")));
		expect(result.current.error).toBe("scan failed");
		expect(result.current.version).toBe(0);
		expect(result.current.changesFileCount).toBe(0);
		expect(result.current.loading).toBe(false);
		act(() =>
			states.publish(target, snapshot({ version: 8, changesFileCount: 2 })),
		);
		expect(result.current.error).toBeNull();
		expect(result.current.changesFileCount).toBe(2);
	});

	it("worktreeやbaseが変わると前の一覧を表示せず新しい対象を購読する", () => {
		const { result, rerender } = renderHook(
			({ path, base }) => useReviewSnapshot(path, base),
			{ initialProps: { path: "/repo", base: "head" as const } },
		);
		act(() =>
			states.publish(
				{ kind: "review-snapshot", args: ["/repo", "head"] },
				snapshot({ version: 7 }),
			),
		);
		expect(result.current.version).toBe(7);

		rerender({ path: "/other", base: "head" });
		expect(result.current.version).toBe(0);
		expect(result.current.loading).toBe(true);
		expect(states.subscribeState).toHaveBeenLastCalledWith(
			{ kind: "review-snapshot", args: ["/other", "head"] },
			expect.any(Function),
			expect.any(Function),
		);
	});

	it("同じ対象の新しい版が届くたびに置き換える", () => {
		const { result } = renderHook(() => useReviewSnapshot("/repo", "head"));
		const target: StateTarget<"review-snapshot"> = {
			kind: "review-snapshot",
			args: ["/repo", "head"],
		};
		act(() => states.publish(target, snapshot({ version: 1 })));
		act(() =>
			states.publish(
				target,
				snapshot({ version: 2, changedFiles: [], changesFileCount: 3 }),
			),
		);
		expect(result.current.version).toBe(2);
		expect(result.current.changesFileCount).toBe(3);
	});
});
