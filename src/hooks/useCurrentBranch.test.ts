import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { subscribeState } from "@/lib/client";
import { useCurrentBranch } from "./useCurrentBranch";

vi.mock("@/lib/client", () => ({ subscribeState: vi.fn() }));
beforeEach(() => vi.clearAllMocks());
it("現在のbranchの変更を購読し対象変更とunmountで解除する", () => {
	let deliver!: (value: string) => void;
	const stop = vi.fn();
	vi.mocked(subscribeState).mockImplementation((_target, receiver) => {
		deliver = receiver;
		return stop;
	});
	const { result, rerender, unmount } = renderHook(
		({ path }) => useCurrentBranch(path),
		{ initialProps: { path: "/repo" } },
	);
	act(() => deliver("main"));
	expect(result.current.branch).toBe("main");
	act(() => deliver("feature"));
	expect(result.current.branch).toBe("feature");
	rerender({ path: "/other" });
	expect(stop).toHaveBeenCalledOnce();
	expect(result.current.branch).toBeNull();
	unmount();
	expect(stop).toHaveBeenCalledTimes(2);
});
it("Repository未選択では購読しない", () => {
	renderHook(() => useCurrentBranch(null));
	expect(subscribeState).not.toHaveBeenCalled();
});

it("読取失敗時は前のブランチを表示せず回復後の値を表示する", () => {
	let receive!: (value: string) => void;
	let fail!: (error: unknown) => void;
	vi.mocked(subscribeState).mockImplementation((_target, next, onError) => {
		receive = next;
		fail = onError;
		return vi.fn();
	});
	const { result } = renderHook(() => useCurrentBranch("/repo"));
	act(() => receive("main"));
	act(() => fail(new Error("denied")));
	expect(result.current).toEqual({ branch: null, error: "denied" });
	act(() => receive("develop"));
	expect(result.current).toEqual({ branch: "develop", error: null });
});
