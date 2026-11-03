import { Code, ConnectError } from "@connectrpc/connect";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceStateDto as WorkspaceState } from "@/generated/client_types";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useWorkspacePersistence } from "./useWorkspacePersistence";
import { useWorkspaceStateCache } from "./useWorkspaceStateCache";

const mockInvoke = vi.fn();
const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

function makeState(overrides?: Partial<WorkspaceState>): WorkspaceState {
	return {
		version: 1,
		tabs: {
			editors: [{ path: "/repo/src/main.rs", name: "main.rs" }],
			activeEditorPath: "/repo/src/main.rs",
		},
		layout: {
			centerTab: "editor",
			activeView: "git",
			leftNavCollapsed: false,
			rightCollapsed: false,
			rightBottomCollapsed: false,
		},
		etag: "1",
		...overrides,
	};
}

describe("useWorkspaceStateCache", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		mockInvoke.mockResolvedValue(makeState({ etag: "2" }));
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
		vi.restoreAllMocks();
	});

	it("daemonが保存を拒否した失敗を画面へ通知する", async () => {
		const failure = new Error("corrupt state");
		mockInvoke.mockRejectedValueOnce(failure);
		const notice = vi.fn();
		window.addEventListener("releash-client-error", notice);
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		const { result, unmount } = renderHook(() => useWorkspaceStateCache());
		try {
			await act(async () => {
				result.current.updateState("/repo", makeState());
				result.current.flushState("/repo");
			});
			expect(mockInvoke).toHaveBeenCalledWith(
				"save_workspace_state",
				expect.anything(),
			);
			expect(log).toHaveBeenCalledWith(
				"Failed to save workspace state:",
				failure,
			);
			expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
				"corrupt state",
			);
		} finally {
			unmount();
			window.removeEventListener("releash-client-error", notice);
		}
	});

	it("同じ末尾名のworktreeを絶対pathで保存・購読し独立して復元する", async () => {
		const paths = ["/repo-worktrees/feature/a", "/repo-worktrees/bugfix/a"];
		const values = [
			makeState(),
			makeState({ tabs: { editors: [], activeEditorPath: null } }),
		];
		const { result } = renderHook(() => useWorkspaceStateCache());
		const received = paths.map(() => vi.fn());
		await act(async () => {
			for (const [index, path] of paths.entries()) {
				result.current.updateState(path, values[index]);
				result.current.flushState(path);
			}
		});
		for (const [index, path] of paths.entries()) {
			expect(mockInvoke).toHaveBeenCalledWith("save_workspace_state", {
				worktreeName: path,
				state: { ...values[index], etag: "0" },
			});
			states.publish(
				{ kind: "workspace-state", args: [path, path] },
				values[index],
			);
		}
		await act(async () => {
			for (const [index, path] of paths.entries())
				result.current.loadState(path, received[index], vi.fn());
		});
		for (const [index, path] of paths.entries()) {
			expect(received[index]).toHaveBeenLastCalledWith(values[index]);
			expect(result.current.getState(path)).toEqual(values[index]);
		}
	});

	it("getState returns undefined for unknown path", () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		expect(result.current.getState("/unknown")).toBeUndefined();
	});

	it("updateState stores state and getState retrieves it", () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const state = makeState();

		act(() => {
			result.current.updateState("/repo", state);
		});

		expect(result.current.getState("/repo")).toEqual(state);
	});

	it("updateState debounces save calls", () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const state1 = makeState();
		const state2 = makeState({
			layout: {
				centerTab: "agent",
				activeView: "git",
				leftNavCollapsed: false,
				rightCollapsed: false,
				rightBottomCollapsed: false,
			},
		});

		act(() => {
			result.current.updateState("/repo", state1);
			result.current.updateState("/repo", state2);
		});

		// No save yet (debounce not elapsed)
		expect(mockInvoke).not.toHaveBeenCalled();

		act(() => {
			vi.advanceTimersByTime(500);
		});

		// Only one save call after debounce
		expect(mockInvoke).toHaveBeenCalledTimes(1);
		expect(mockInvoke).toHaveBeenCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...state2, etag: "0" },
		});
	});

	it("flushState saves immediately and cancels pending debounce", () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const state = makeState();

		act(() => {
			result.current.updateState("/repo", state);
		});
		expect(mockInvoke).not.toHaveBeenCalled();

		act(() => {
			result.current.flushState("/repo");
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);
		expect(mockInvoke).toHaveBeenCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...state, etag: "0" },
		});

		// Advancing timers should not trigger another save
		act(() => {
			vi.advanceTimersByTime(1000);
		});
		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("未応答の保存はflushで再送し、成功後はflushとunmountで再送しない", async () => {
		let resolveSave!: (saved: WorkspaceState) => void;
		mockInvoke.mockReturnValueOnce(
			new Promise<WorkspaceState>((resolve) => {
				resolveSave = resolve;
			}),
		);
		const { result, unmount } = renderHook(() => useWorkspaceStateCache());
		const state = makeState();

		act(() => {
			result.current.updateState("/repo", state);
			vi.advanceTimersByTime(500);
		});
		expect(mockInvoke).toHaveBeenCalledTimes(1);
		await act(async () => {
			result.current.flushState("/repo");
		});
		expect(mockInvoke).toHaveBeenCalledTimes(2);
		expect(mockInvoke).toHaveBeenLastCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...state, etag: "0" },
		});
		await act(async () => resolveSave(makeState({ etag: "2" })));
		act(() => result.current.flushState("/repo"));
		unmount();
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("先行保存の成功は、後続の保存が失敗した最新状態の未保存マークを消さない", async () => {
		let resolveSave!: (saved: WorkspaceState) => void;
		mockInvoke.mockReturnValueOnce(
			new Promise<WorkspaceState>((resolve) => {
				resolveSave = resolve;
			}),
		);
		const error = new Error("Client WebSocket connection closed");
		const notice = vi.fn();
		window.addEventListener("releash-client-error", notice);
		mockInvoke.mockRejectedValueOnce(error);
		const logError = vi.spyOn(console, "error").mockImplementation(() => {});
		const { result, unmount } = renderHook(() => useWorkspaceStateCache());
		const latest = makeState({ tabs: { editors: [], activeEditorPath: null } });

		act(() => {
			result.current.updateState("/repo", makeState());
			result.current.flushState("/repo");
		});
		await act(async () => {
			result.current.updateState("/repo", latest);
			result.current.flushState("/repo");
		});
		expect(logError).toHaveBeenCalledWith(
			"Failed to save workspace state:",
			error,
		);
		expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
			error.message,
		);
		window.removeEventListener("releash-client-error", notice);
		await act(async () => resolveSave(makeState({ etag: "5" })));
		await act(async () => result.current.flushState("/repo"));
		expect(mockInvoke).toHaveBeenCalledTimes(3);
		expect(mockInvoke).toHaveBeenLastCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...latest, etag: "5" },
		});
		unmount();
		expect(mockInvoke).toHaveBeenCalledTimes(3);
	});

	it("保存が拒否されたら編集を捨てて購読の最新を読み直し、次の保存はその版で送る", async () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const received = vi.fn();
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		const target = {
			kind: "workspace-state" as const,
			args: ["/repo", "/repo"],
		};
		act(() => {
			result.current.loadState("/repo", received, vi.fn());
			states.publish(target, makeState({ etag: "1" }));
		});
		const edited = makeState({ tabs: { editors: [], activeEditorPath: null } });
		const latest = makeState({ etag: "2", layout: undefined });
		mockInvoke.mockRejectedValueOnce(new ConnectError("stale", Code.Aborted));
		await act(async () => {
			result.current.updateState("/repo", edited);
			states.publish(target, latest);
			result.current.flushState("/repo");
		});
		expect(mockInvoke).toHaveBeenLastCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...edited, etag: "1" },
		});
		expect(result.current.getState("/repo")).toEqual(latest);
		expect(received).toHaveBeenLastCalledWith(latest);
		expect(log).not.toHaveBeenCalled();
		await act(async () => {
			result.current.updateState("/repo", edited);
			result.current.flushState("/repo");
		});
		expect(mockInvoke).toHaveBeenLastCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...edited, etag: "2" },
		});
	});

	it("loadState subscribes and caches current and changed values", async () => {
		const state = makeState();
		states.publish(
			{ kind: "workspace-state", args: ["/repo", "/repo"] },
			state,
		);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: import("@/types/workspace-state").WorkspaceState | undefined;
		await act(async () => {
			result.current.loadState(
				"/repo",
				(state) => {
					loaded = state;
				},
				vi.fn(),
			);
		});

		expect(loaded).toEqual(state);
		expect(result.current.getState("/repo")).toEqual(state);
		expect(states.subscribeState).toHaveBeenCalledWith(
			{ kind: "workspace-state", args: ["/repo", "/repo"] },
			expect.any(Function),
			expect.any(Function),
		);
		const next = makeState({ tabs: { editors: [], activeEditorPath: null } });
		act(() =>
			states.publish(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				next,
			),
		);
		expect(result.current.getState("/repo")).toEqual(next);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("tabsとlayoutがない購読値を保持し、画面が更新した値をそのまま保存する", async () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const received = vi.fn();
		const failed = vi.fn();
		act(() => {
			result.current.loadState("/repo", received, failed);
			states.publish(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				{ version: 1, etag: "3" },
			);
		});
		expect(received).toHaveBeenLastCalledWith({ version: 1, etag: "3" });
		expect(result.current.getState("/repo")).toEqual({ version: 1, etag: "3" });
		expect(failed).not.toHaveBeenCalled();
		expect(mockInvoke).not.toHaveBeenCalled();
		const state = makeState();
		await act(async () => {
			result.current.updateState("/repo", state);
			result.current.flushState("/repo");
		});
		expect(mockInvoke).toHaveBeenCalledWith("save_workspace_state", {
			worktreeName: "/repo",
			state: { ...state, etag: "3" },
		});
	});

	it("loadState returns undefined when backend returns null", async () => {
		states.publish({ kind: "workspace-state", args: ["/repo", "/repo"] }, null);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: import("@/types/workspace-state").WorkspaceState | undefined;
		await act(async () => {
			result.current.loadState(
				"/repo",
				(state) => {
					loaded = state;
				},
				vi.fn(),
			);
		});

		expect(loaded).toBeUndefined();
		expect(result.current.getState("/repo")).toBeUndefined();
	});
	it("保存済み表示状態の購読失敗は不在と区別して失敗のcallbackへ渡す", async () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const received = vi.fn();
		const failed = vi.fn();
		result.current.loadState("/repo", received, failed);
		states.fail(
			{ kind: "workspace-state", args: ["/repo", "/repo"] },
			new Error("offline"),
		);
		expect(failed).toHaveBeenCalledWith(new Error("offline"));
		expect(received).not.toHaveBeenCalled();
		expect(result.current.getState("/repo")).toBeUndefined();
		expect(states.subscribeState).toHaveBeenCalledTimes(1);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("購読失敗では既定状態を復元済みと扱わず原因を返す", async () => {
		const { result } = renderHook(() =>
			useWorkspacePersistence({
				selectedRootPath: "/repo",
				centerTab: "editor",
				leftNavVisible: true,
				rightVisible: true,
				setCenterTab: vi.fn(),
				leftNavRef: { current: null },
				rightPanelRef: { current: null },
			}),
		);
		expect(result.current.stateReady).toBe(false);
		await act(async () =>
			states.fail(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				new Error("offline"),
			),
		);
		expect(result.current.stateReady).toBe(false);
		expect(result.current.stateError).toBe("offline");
		await act(async () =>
			states.publish(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				makeState(),
			),
		);
		expect(result.current.stateError).toBeNull();
		expect(result.current.stateReady).toBe(true);
		expect(states.subscribeState).toHaveBeenCalledTimes(1);
	});
	it("初回失敗後の正常な値なしでも同じ購読で復旧する", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		const { result } = renderHook(() =>
			useWorkspacePersistence({
				selectedRootPath: "/repo",
				centerTab: "agent",
				leftNavVisible: true,
				rightVisible: true,
				setCenterTab: vi.fn(),
				leftNavRef: { current: null },
				rightPanelRef: { current: null },
			}),
		);
		await act(async () =>
			states.fail(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				new Error("offline"),
			),
		);
		expect(result.current.stateReady).toBe(false);
		expect(result.current.stateError).toBe("offline");
		await act(async () =>
			states.publish(
				{ kind: "workspace-state", args: ["/repo", "/repo"] },
				null,
			),
		);
		expect(result.current.stateReady).toBe(true);
		expect(result.current.stateError).toBeNull();
		expect(states.subscribeState).toHaveBeenCalledTimes(1);
	});
});
