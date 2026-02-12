import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { GitFileStatus } from "@/types/git";
import { useGitStatus } from "./useGitStatus";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

describe("useGitStatus", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("should return empty data when rootPath is null", () => {
		mockInvoke.mockResolvedValue([]);
		const { result } = renderHook(() => useGitStatus(null));

		expect(result.current.statusMap.size).toBe(0);
		expect(result.current.stagedFiles).toEqual([]);
		expect(result.current.changedFiles).toEqual([]);
	});

	it("should fetch and map git status", async () => {
		const mockEntries: GitFileStatus[] = [
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
			{ path: "new_file.txt", index_status: "none", worktree_status: "new" },
			{ path: "staged.txt", index_status: "new", worktree_status: "none" },
		];
		mockInvoke.mockResolvedValue(mockEntries);

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(3);
		});

		expect(result.current.statusMap.get("/test/repo/src/main.ts")).toBe(
			"modified",
		);
		expect(result.current.statusMap.get("/test/repo/new_file.txt")).toBe(
			"untracked",
		);
		expect(result.current.statusMap.get("/test/repo/staged.txt")).toBe("added");

		expect(result.current.changedFiles).toHaveLength(2);
		expect(result.current.stagedFiles).toHaveLength(1);
	});

	it("should map deleted worktree status", async () => {
		const mockEntries: GitFileStatus[] = [
			{ path: "deleted.txt", index_status: "none", worktree_status: "deleted" },
		];
		mockInvoke.mockResolvedValue(mockEntries);

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(1);
		});

		expect(result.current.statusMap.get("/test/repo/deleted.txt")).toBe(
			"deleted",
		);
	});

	it("should map index-only statuses when worktree is none", async () => {
		const mockEntries: GitFileStatus[] = [
			{
				path: "modified_idx.txt",
				index_status: "modified",
				worktree_status: "none",
			},
			{
				path: "deleted_idx.txt",
				index_status: "deleted",
				worktree_status: "none",
			},
			{
				path: "renamed_idx.txt",
				index_status: "renamed",
				worktree_status: "none",
			},
		];
		mockInvoke.mockResolvedValue(mockEntries);

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(3);
		});

		expect(result.current.statusMap.get("/test/repo/modified_idx.txt")).toBe(
			"modified",
		);
		expect(result.current.statusMap.get("/test/repo/deleted_idx.txt")).toBe(
			"deleted",
		);
		expect(result.current.statusMap.get("/test/repo/renamed_idx.txt")).toBe(
			"modified",
		);
	});

	it("should map ignored status and exclude from changedFiles", async () => {
		const mockEntries: GitFileStatus[] = [
			{
				path: "node_modules",
				index_status: "none",
				worktree_status: "ignored",
			},
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		mockInvoke.mockResolvedValue(mockEntries);

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(2);
		});

		expect(result.current.statusMap.get("/test/repo/node_modules")).toBe(
			"ignored",
		);
		expect(result.current.statusMap.get("/test/repo/src/main.ts")).toBe(
			"modified",
		);
		expect(result.current.changedFiles).toHaveLength(1);
		expect(result.current.changedFiles[0].path).toBe("src/main.ts");
	});

	it("should handle invoke error gracefully", async () => {
		mockInvoke.mockRejectedValue(new Error("not a git repo"));

		const { result } = renderHook(() => useGitStatus("/test/not-repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalled();
		});

		expect(result.current.statusMap.size).toBe(0);
		expect(result.current.stagedFiles).toEqual([]);
		expect(result.current.changedFiles).toEqual([]);
	});

	it("should debounce refresh on file-change events", async () => {
		vi.useFakeTimers();
		mockInvoke.mockResolvedValue([]);

		type ListenCallback = (event: { payload: { path: string } }) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo"));

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		expect(fileChangeCallback).not.toBeNull();

		act(() => {
			fileChangeCallback?.({ payload: { path: "/test/repo/src/a.ts" } });
			fileChangeCallback?.({ payload: { path: "/test/repo/src/b.ts" } });
			fileChangeCallback?.({ payload: { path: "/test/repo/src/c.ts" } });
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(mockInvoke).toHaveBeenCalledTimes(2);

		vi.useRealTimers();
	});

	it("should ignore file-change events from different rootPath", async () => {
		vi.useFakeTimers();
		mockInvoke.mockResolvedValue([]);

		type ListenCallback = (event: { payload: { path: string } }) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo-a"));

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		act(() => {
			fileChangeCallback?.({ payload: { path: "/test/repo-b/src/file.ts" } });
		});

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);

		vi.useRealTimers();
	});

	it("should cancel pending debounce when externalRefreshKey changes", async () => {
		vi.useFakeTimers();
		mockInvoke.mockResolvedValue([]);

		type ListenCallback = (event: { payload: { path: string } }) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		const { rerender } = renderHook(
			({ refreshKey }: { refreshKey: number }) =>
				useGitStatus("/test/repo", refreshKey),
			{ initialProps: { refreshKey: 0 } },
		);

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		act(() => {
			fileChangeCallback?.({ payload: { path: "/test/repo/src/a.ts" } });
		});

		rerender({ refreshKey: 1 });

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(2);
		});

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(mockInvoke).toHaveBeenCalledTimes(2);

		vi.useRealTimers();
	});

	it("should re-fetch when refresh is called", async () => {
		mockInvoke.mockResolvedValue([]);

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		act(() => {
			result.current.refresh();
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(2);
		});
	});
});
