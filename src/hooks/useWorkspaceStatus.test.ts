import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceStatus } from "@/types/session";
import { useWorkspaceStatus } from "./useWorkspaceStatus";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeStatus = (
	overrides: Partial<WorkspaceStatus> = {},
): WorkspaceStatus => ({
	worktree_id: "/tmp/wt",
	worktree_path: "/tmp/wt",
	aggregated_state: "running",
	running_count: 1,
	waiting_count: 0,
	error_count: 0,
	session_count: 1,
	last_activity_at: 1_000,
	...overrides,
});

describe("useWorkspaceStatus", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns null when worktreeId is null", () => {
		const { result } = renderHook(() => useWorkspaceStatus(null));
		expect(result.current).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("fetches initial status via get_workspace_status", async () => {
		const status = makeStatus();
		mockInvoke.mockResolvedValue(status);

		const { result } = renderHook(() => useWorkspaceStatus("/tmp/wt"));

		await waitFor(() => {
			expect(result.current).toEqual(status);
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_workspace_status", {
			worktreeId: "/tmp/wt",
		});
	});

	it("updates status when matching workspace-status-changed fires", async () => {
		mockInvoke.mockResolvedValue(makeStatus({ aggregated_state: "done" }));

		type Cb = (event: { payload: WorkspaceStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workspace-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkspaceStatus("/tmp/wt"));
		await waitFor(() => {
			expect(result.current?.aggregated_state).toBe("done");
		});

		await act(async () => {
			cb?.({ payload: makeStatus({ aggregated_state: "error" }) });
		});

		expect(result.current?.aggregated_state).toBe("error");
	});

	it("ignores events for other worktrees", async () => {
		mockInvoke.mockResolvedValue(makeStatus({ aggregated_state: "done" }));

		type Cb = (event: { payload: WorkspaceStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workspace-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkspaceStatus("/tmp/wt"));
		await waitFor(() => {
			expect(result.current?.aggregated_state).toBe("done");
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({
					worktree_id: "/tmp/other",
					aggregated_state: "error",
				}),
			});
		});

		expect(result.current?.aggregated_state).toBe("done");
	});

	it("returns null when invoke fails", async () => {
		mockInvoke.mockRejectedValue(new Error("not found"));
		const { result } = renderHook(() => useWorkspaceStatus("/tmp/wt"));
		await waitFor(() => {
			expect(result.current).toBeNull();
		});
	});
});
