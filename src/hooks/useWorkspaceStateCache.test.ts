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
		...overrides,
	};
}

describe("useWorkspaceStateCache", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		mockInvoke.mockResolvedValue(undefined);
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
		vi.restoreAllMocks();
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
				...makeState().layout,
				centerTab: "agent",
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
			worktreeName: "repo",
			state: state2,
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
			worktreeName: "repo",
			state,
		});

		// Advancing timers should not trigger another save
		act(() => {
			vi.advanceTimersByTime(1000);
		});
		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("未応答の保存はflushで再送し、成功後はflushとunmountで再送しない", async () => {
		let resolveSave!: () => void;
		mockInvoke.mockReturnValueOnce(
			new Promise<void>((resolve) => {
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
			worktreeName: "repo",
			state,
		});
		await act(async () => resolveSave());
		act(() => result.current.flushState("/repo"));
		unmount();
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("先行保存の成功は、後続の保存が失敗した最新状態の未保存マークを消さない", async () => {
		let resolveSave!: () => void;
		mockInvoke.mockReturnValueOnce(
			new Promise<void>((resolve) => {
				resolveSave = resolve;
			}),
		);
		const error = new Error("Client WebSocket connection closed");
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
		await act(async () => resolveSave());
		await act(async () => result.current.flushState("/repo"));
		expect(mockInvoke).toHaveBeenCalledTimes(3);
		expect(mockInvoke).toHaveBeenLastCalledWith("save_workspace_state", {
			worktreeName: "repo",
			state: latest,
		});
		unmount();
		expect(mockInvoke).toHaveBeenCalledTimes(3);
	});

	it("loadState subscribes and caches current and changed values", async () => {
		const state = makeState();
		states.publish({ kind: "workspace-state", args: ["repo", "/repo"] }, state);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: import("@/types/workspace-state").WorkspaceState | undefined;
		await act(async () => {
			loaded = await result.current.loadState("/repo");
		});

		expect(loaded).toEqual(state);
		expect(result.current.getState("/repo")).toEqual(state);
		expect(states.subscribeState).toHaveBeenCalledWith(
			{ kind: "workspace-state", args: ["repo", "/repo"] },
			expect.any(Function),
			expect.any(Function),
		);
		const next = makeState({ tabs: { editors: [], activeEditorPath: null } });
		act(() =>
			states.publish(
				{ kind: "workspace-state", args: ["repo", "/repo"] },
				next,
			),
		);
		expect(result.current.getState("/repo")).toEqual(next);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("loadState returns undefined when backend returns null", async () => {
		states.publish({ kind: "workspace-state", args: ["repo", "/repo"] }, null);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: import("@/types/workspace-state").WorkspaceState | undefined;
		await act(async () => {
			loaded = await result.current.loadState("/repo");
		});

		expect(loaded).toBeUndefined();
		expect(result.current.getState("/repo")).toBeUndefined();
	});
	it("保存済み表示状態の購読失敗をundefinedで完了し再試行しない", async () => {
		const { result } = renderHook(() => useWorkspaceStateCache());
		const loaded = result.current.loadState("/repo");
		states.fail(
			{ kind: "workspace-state", args: ["repo", "/repo"] },
			new Error("offline"),
		);
		await expect(loaded).resolves.toBeUndefined();
		expect(states.subscribeState).toHaveBeenCalledTimes(1);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("購読失敗でも呼び出し元のstateReadyが完了する", async () => {
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
				{ kind: "workspace-state", args: ["repo", "/repo"] },
				new Error("offline"),
			),
		);
		expect(result.current.stateReady).toBe(true);
		expect(states.subscribeState).toHaveBeenCalledTimes(1);
	});
});
