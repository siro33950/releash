import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useBrowserBackGuard } from "./useBrowserBackGuard";

describe("useBrowserBackGuard", () => {
	const originalReplaceState = history.replaceState;
	const originalPushState = history.pushState;
	let replaceStateSpy: ReturnType<typeof vi.fn>;
	let pushStateSpy: ReturnType<typeof vi.fn>;
	let popStateListeners: Array<(e: PopStateEvent) => void>;

	beforeEach(() => {
		popStateListeners = [];
		replaceStateSpy = vi.fn();
		pushStateSpy = vi.fn();
		history.replaceState = replaceStateSpy;
		history.pushState = pushStateSpy;

		vi.spyOn(window, "addEventListener").mockImplementation(
			(event: string, handler: unknown) => {
				if (event === "popstate") {
					popStateListeners.push(handler as (e: PopStateEvent) => void);
				}
			},
		);
		vi.spyOn(window, "removeEventListener").mockImplementation(
			(event: string, handler: unknown) => {
				if (event === "popstate") {
					popStateListeners = popStateListeners.filter((h) => h !== handler);
				}
			},
		);
	});

	afterEach(() => {
		history.replaceState = originalReplaceState;
		history.pushState = originalPushState;
		vi.restoreAllMocks();
	});

	function firePopState(state: unknown = null) {
		const event = new PopStateEvent("popstate", { state });
		for (const listener of popStateListeners) {
			listener(event);
		}
	}

	it("マウント時にガードエントリをブラウザ履歴に追加する", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		expect(replaceStateSpy).toHaveBeenCalledWith({ _remoteGuard: true }, "");
		expect(pushStateSpy).toHaveBeenCalledWith({ _remoteGuard: true }, "");
	});

	it("Worktree選択時に履歴エントリを追加する", () => {
		const onBack = vi.fn();
		const { rerender } = renderHook(
			({ worktree }) =>
				useBrowserBackGuard({ selectedWorktree: worktree, onBack }),
			{ initialProps: { worktree: null as string | null } },
		);

		pushStateSpy.mockClear();

		rerender({ worktree: "/path/to/worktree" });

		expect(pushStateSpy).toHaveBeenCalledWith({ _remoteWorktree: true }, "");
	});

	it("Worktree表示中のブラウザバックでonBackを呼び出す", () => {
		const onBack = vi.fn();
		renderHook(() =>
			useBrowserBackGuard({
				selectedWorktree: "/path/to/worktree",
				onBack,
			}),
		);

		pushStateSpy.mockClear();
		act(() => firePopState());

		expect(onBack).toHaveBeenCalledOnce();
		expect(pushStateSpy).toHaveBeenCalledWith({ _remoteGuard: true }, "");
	});

	it("ダッシュボード表示中のブラウザバックではonBackを呼ばずガードを再追加する", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		pushStateSpy.mockClear();
		act(() => firePopState());

		expect(onBack).not.toHaveBeenCalled();
		expect(pushStateSpy).toHaveBeenCalledWith({ _remoteGuard: true }, "");
	});

	it("navigateBackがhistory.back()を呼び出す", () => {
		const onBack = vi.fn();
		const backSpy = vi.spyOn(history, "back").mockImplementation(() => {});
		const { result } = renderHook(() =>
			useBrowserBackGuard({ selectedWorktree: null, onBack }),
		);

		act(() => result.current.navigateBack());

		expect(backSpy).toHaveBeenCalledOnce();
		backSpy.mockRestore();
	});

	it("selectedWorktreeがnullの場合は履歴エントリを追加しない", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		// マウント時のガード以外のpushStateが無いことを確認
		const worktreePushes = pushStateSpy.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as Record<string, unknown>)._remoteWorktree === true,
		);
		expect(worktreePushes).toHaveLength(0);
	});

	it("アンマウント時にpopstateリスナーを解除する", () => {
		const onBack = vi.fn();
		const { unmount } = renderHook(() =>
			useBrowserBackGuard({ selectedWorktree: null, onBack }),
		);

		expect(popStateListeners).toHaveLength(1);

		unmount();

		expect(popStateListeners).toHaveLength(0);
	});

	it("onBackコールバックが変更されても最新のものが呼ばれる", () => {
		const onBack1 = vi.fn();
		const onBack2 = vi.fn();
		const { rerender } = renderHook(
			({ onBack }) =>
				useBrowserBackGuard({
					selectedWorktree: "/path/to/worktree",
					onBack,
				}),
			{ initialProps: { onBack: onBack1 } },
		);

		rerender({ onBack: onBack2 });

		act(() => firePopState());

		expect(onBack1).not.toHaveBeenCalled();
		expect(onBack2).toHaveBeenCalledOnce();
	});
});
