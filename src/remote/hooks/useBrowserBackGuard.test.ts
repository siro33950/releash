import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useBrowserBackGuard } from "./useBrowserBackGuard";

// biome-ignore lint/suspicious/noExplicitAny: テスト用のスパイ型
let replaceStateSpy: any;
// biome-ignore lint/suspicious/noExplicitAny: テスト用のスパイ型
let pushStateSpy: any;

beforeEach(() => {
	replaceStateSpy = vi
		.spyOn(history, "replaceState")
		.mockImplementation(() => {});
	pushStateSpy = vi.spyOn(history, "pushState").mockImplementation(() => {});
});

afterEach(() => {
	vi.restoreAllMocks();
	// biome-ignore lint/suspicious/noExplicitAny: テスト用のwindow操作
	delete (window as any).navigation;
});

describe("useBrowserBackGuard 共通", () => {
	it("マウント時にscrollRestorationをmanualに設定する", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		expect(history.scrollRestoration).toBe("manual");
	});

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

	it("selectedWorktreeがnullの場合は履歴エントリを追加しない", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		const worktreePushes = pushStateSpy.mock.calls.filter(
			(call: unknown[]) =>
				(call[0] as Record<string, unknown>)._remoteWorktree === true,
		);
		expect(worktreePushes).toHaveLength(0);
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
});

describe("useBrowserBackGuard popstateフォールバック", () => {
	let popStateListeners: Array<(e: PopStateEvent) => void>;

	beforeEach(() => {
		popStateListeners = [];
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

	function firePopState() {
		const event = new PopStateEvent("popstate", { state: null });
		for (const listener of popStateListeners) {
			listener(event);
		}
	}

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

describe("useBrowserBackGuard Navigation API", () => {
	// biome-ignore lint/suspicious/noExplicitAny: テスト用のモック
	let navigateListeners: Array<(event: any) => void>;

	beforeEach(() => {
		navigateListeners = [];
		// biome-ignore lint/suspicious/noExplicitAny: テスト用のwindow操作
		(window as any).navigation = {
			addEventListener: (type: string, handler: unknown) => {
				if (type === "navigate") {
					navigateListeners.push(handler as (event: unknown) => void);
				}
			},
			removeEventListener: (type: string, handler: unknown) => {
				if (type === "navigate") {
					navigateListeners = navigateListeners.filter((h) => h !== handler);
				}
			},
		};
	});

	function fireNavigateTraverse() {
		let intercepted = false;
		// biome-ignore lint/suspicious/noExplicitAny: テスト用のモック
		let interceptHandler: any;
		const event = {
			navigationType: "traverse",
			canIntercept: true,
			intercept: (opts: { handler?: () => Promise<void> }) => {
				intercepted = true;
				interceptHandler = opts?.handler;
			},
		};
		for (const listener of navigateListeners) {
			listener(event);
		}
		return { intercepted, interceptHandler };
	}

	it("Navigation API対応時にnavigateリスナーを登録する", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		expect(navigateListeners).toHaveLength(1);
	});

	it("traverseナビゲーションをインターセプトする", () => {
		const onBack = vi.fn();
		renderHook(() =>
			useBrowserBackGuard({
				selectedWorktree: "/path/to/worktree",
				onBack,
			}),
		);

		const { intercepted } = fireNavigateTraverse();
		expect(intercepted).toBe(true);
	});

	it("Worktree表示中のtraverseでonBackを呼び出す", async () => {
		const onBack = vi.fn();
		renderHook(() =>
			useBrowserBackGuard({
				selectedWorktree: "/path/to/worktree",
				onBack,
			}),
		);

		const { interceptHandler } = fireNavigateTraverse();
		await interceptHandler();

		expect(onBack).toHaveBeenCalledOnce();
	});

	it("ダッシュボード表示中のtraverseではonBackを呼ばない", async () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		const { interceptHandler } = fireNavigateTraverse();
		await interceptHandler();

		expect(onBack).not.toHaveBeenCalled();
	});

	it("アンマウント時にnavigateリスナーを解除する", () => {
		const onBack = vi.fn();
		const { unmount } = renderHook(() =>
			useBrowserBackGuard({ selectedWorktree: null, onBack }),
		);

		expect(navigateListeners).toHaveLength(1);
		unmount();
		expect(navigateListeners).toHaveLength(0);
	});

	it("traverse以外のナビゲーションはインターセプトしない", () => {
		const onBack = vi.fn();
		renderHook(() => useBrowserBackGuard({ selectedWorktree: null, onBack }));

		let intercepted = false;
		const event = {
			navigationType: "push",
			canIntercept: true,
			intercept: () => {
				intercepted = true;
			},
		};
		for (const listener of navigateListeners) {
			listener(event);
		}
		expect(intercepted).toBe(false);
	});
});
