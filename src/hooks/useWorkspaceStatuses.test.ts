import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceStatus } from "@/types/session";
import { useWorkspaceStatuses } from "./useWorkspaceStatuses";

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

describe("useWorkspaceStatuses", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("fetches initial statuses via list_workspace_statuses", async () => {
		const a = makeStatus({ worktree_id: "/tmp/a" });
		const b = makeStatus({ worktree_id: "/tmp/b", aggregated_state: "done" });
		mockInvoke.mockResolvedValue([a, b]);

		const { result } = renderHook(() => useWorkspaceStatuses());

		await waitFor(() => {
			expect(Object.keys(result.current).length).toBe(2);
		});
		expect(result.current["/tmp/a"]).toEqual(a);
		expect(result.current["/tmp/b"]).toEqual(b);
	});

	it("merges new entries from workspace-status-changed", async () => {
		mockInvoke.mockResolvedValue([makeStatus({ worktree_id: "/tmp/a" })]);

		type Cb = (event: { payload: WorkspaceStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workspace-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkspaceStatuses());

		await waitFor(() => {
			expect(result.current["/tmp/a"]).toBeDefined();
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({
					worktree_id: "/tmp/b",
					aggregated_state: "error",
				}),
			});
		});

		expect(result.current["/tmp/b"]?.aggregated_state).toBe("error");
		expect(result.current["/tmp/a"]).toBeDefined();
	});

	it("overwrites existing entry from workspace-status-changed", async () => {
		mockInvoke.mockResolvedValue([
			makeStatus({ worktree_id: "/tmp/a", aggregated_state: "done" }),
		]);

		type Cb = (event: { payload: WorkspaceStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "workspace-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorkspaceStatuses());

		await waitFor(() => {
			expect(result.current["/tmp/a"]?.aggregated_state).toBe("done");
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({
					worktree_id: "/tmp/a",
					aggregated_state: "running",
				}),
			});
		});

		expect(result.current["/tmp/a"]?.aggregated_state).toBe("running");
	});

	it("returns empty record when invoke fails", async () => {
		mockInvoke.mockRejectedValue(new Error("boom"));
		const { result } = renderHook(() => useWorkspaceStatuses());
		await waitFor(() => {
			expect(result.current).toEqual({});
		});
	});
});
