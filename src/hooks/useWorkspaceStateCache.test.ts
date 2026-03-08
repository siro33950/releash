import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceState } from "@/types/workspace-state";
import { useWorkspaceStateCache } from "./useWorkspaceStateCache";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
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
			rightBottomActiveTab: "terminal",
		},
		...overrides,
	};
}

describe("useWorkspaceStateCache", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockResolvedValue(undefined);
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
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

	it("loadState calls invoke and caches result", async () => {
		const state = makeState();
		mockInvoke.mockResolvedValue(state);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: WorkspaceState | undefined;
		await act(async () => {
			loaded = await result.current.loadState("/repo");
		});

		expect(loaded).toEqual(state);
		expect(result.current.getState("/repo")).toEqual(state);
		expect(mockInvoke).toHaveBeenCalledWith("load_workspace_state", {
			worktreeName: "repo",
			worktreeRoot: "/repo",
		});
	});

	it("loadState returns undefined when backend returns null", async () => {
		mockInvoke.mockResolvedValue(null);

		const { result } = renderHook(() => useWorkspaceStateCache());

		let loaded: WorkspaceState | undefined;
		await act(async () => {
			loaded = await result.current.loadState("/repo");
		});

		expect(loaded).toBeUndefined();
		expect(result.current.getState("/repo")).toBeUndefined();
	});
});
