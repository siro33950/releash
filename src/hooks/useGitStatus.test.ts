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

function gitStatusCallCount(): number {
	return mockInvoke.mock.calls.filter((call) => call[0] === "get_git_status")
		.length;
}

describe("useGitStatus", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});
	});

	it("should return empty data when rootPath is null", () => {
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
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

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
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

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
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

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
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

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
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status")
				return Promise.reject(new Error("not a git repo"));
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		const { result } = renderHook(() => useGitStatus("/test/not-repo"));

		await waitFor(() => {
			expect(gitStatusCallCount()).toBeGreaterThanOrEqual(1);
		});

		expect(result.current.statusMap.size).toBe(0);
		expect(result.current.stagedFiles).toEqual([]);
		expect(result.current.changedFiles).toEqual([]);
	});

	it("should debounce refresh on file-change events", async () => {
		vi.useFakeTimers();

		const WATCHER_ID = 42;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve([]);
			if (cmd === "start_watching") return Promise.resolve(WATCHER_ID);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		type ListenCallback = (event: {
			payload: { watcher_id: number; path: string };
		}) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo"));

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_watching", {
				path: "/test/repo",
			});
		});

		expect(fileChangeCallback).not.toBeNull();

		act(() => {
			fileChangeCallback?.({
				payload: { watcher_id: WATCHER_ID, path: "/test/repo/src/a.ts" },
			});
			fileChangeCallback?.({
				payload: { watcher_id: WATCHER_ID, path: "/test/repo/src/b.ts" },
			});
			fileChangeCallback?.({
				payload: { watcher_id: WATCHER_ID, path: "/test/repo/src/c.ts" },
			});
		});

		expect(gitStatusCallCount()).toBe(1);

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(gitStatusCallCount()).toBe(2);

		vi.useRealTimers();
	});

	it("should ignore file-change events with non-matching watcher_id", async () => {
		vi.useFakeTimers();

		const WATCHER_ID = 42;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve([]);
			if (cmd === "start_watching") return Promise.resolve(WATCHER_ID);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		type ListenCallback = (event: {
			payload: { watcher_id: number; path: string };
		}) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo-a"));

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_watching", {
				path: "/test/repo-a",
			});
		});

		act(() => {
			fileChangeCallback?.({
				payload: { watcher_id: 999, path: "/test/repo-b/src/file.ts" },
			});
		});

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(gitStatusCallCount()).toBe(1);

		vi.useRealTimers();
	});

	it("should deduplicate when debounce fires after externalRefreshKey fetch", async () => {
		vi.useFakeTimers();

		const WATCHER_ID = 42;
		const mockEntries: GitFileStatus[] = [
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(WATCHER_ID);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		type ListenCallback = (event: {
			payload: { watcher_id: number; path: string };
		}) => void;
		let fileChangeCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "file-change") {
				fileChangeCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		const { result, rerender } = renderHook(
			({ refreshKey }: { refreshKey: number }) =>
				useGitStatus("/test/repo", refreshKey),
			{ initialProps: { refreshKey: 0 } },
		);

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});
		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_watching", {
				path: "/test/repo",
			});
		});

		act(() => {
			fileChangeCallback?.({
				payload: { watcher_id: WATCHER_ID, path: "/test/repo/src/a.ts" },
			});
		});

		rerender({ refreshKey: 1 });

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(2);
		});

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(gitStatusCallCount()).toBe(3);
		expect(result.current.statusMap.size).toBe(1);

		vi.useRealTimers();
	});

	it("should re-fetch when refresh is called", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve([]);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});

		act(() => {
			result.current.refresh();
		});

		await waitFor(() => {
			expect(gitStatusCallCount()).toBe(2);
		});
	});

	it("should skip state update when entries are identical to previous fetch", async () => {
		const mockEntries: GitFileStatus[] = [
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(mockEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(1);
		});

		const firstStatusMap = result.current.statusMap;
		const firstChangedFiles = result.current.changedFiles;

		act(() => {
			result.current.refresh();
		});

		await waitFor(() => {
			expect(gitStatusCallCount()).toBe(2);
		});

		expect(result.current.statusMap).toBe(firstStatusMap);
		expect(result.current.changedFiles).toBe(firstChangedFiles);
	});

	it("should update state when entries change between fetches", async () => {
		const initialEntries: GitFileStatus[] = [
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
		];
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(initialEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		const { result } = renderHook(() => useGitStatus("/test/repo"));

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(1);
		});

		const firstStatusMap = result.current.statusMap;

		const updatedEntries: GitFileStatus[] = [
			{
				path: "src/main.ts",
				index_status: "none",
				worktree_status: "modified",
			},
			{
				path: "src/new.ts",
				index_status: "none",
				worktree_status: "new",
			},
		];
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve(updatedEntries);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		act(() => {
			result.current.refresh();
		});

		await waitFor(() => {
			expect(result.current.statusMap.size).toBe(2);
		});

		expect(result.current.statusMap).not.toBe(firstStatusMap);
	});

	it("should debounce refresh on git-status-changed events", async () => {
		vi.useFakeTimers();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve([]);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		type GitStatusCallback = (event: {
			payload: { repo_path: string };
		}) => void;
		let gitStatusCallback: GitStatusCallback | null = null;
		mockListen.mockImplementation((event: string, cb: GitStatusCallback) => {
			if (event === "git-status-changed") {
				gitStatusCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo"));

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});

		expect(gitStatusCallback).not.toBeNull();

		act(() => {
			gitStatusCallback?.({ payload: { repo_path: "/test/repo" } });
		});

		expect(gitStatusCallCount()).toBe(1);

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(gitStatusCallCount()).toBe(2);

		vi.useRealTimers();
	});

	it("should ignore git-status-changed events from different repo_path", async () => {
		vi.useFakeTimers();
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_git_status") return Promise.resolve([]);
			if (cmd === "start_watching") return Promise.resolve(1);
			if (cmd === "stop_watching") return Promise.resolve();
			return Promise.resolve([]);
		});

		type GitStatusCallback = (event: {
			payload: { repo_path: string };
		}) => void;
		let gitStatusCallback: GitStatusCallback | null = null;
		mockListen.mockImplementation((event: string, cb: GitStatusCallback) => {
			if (event === "git-status-changed") {
				gitStatusCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		renderHook(() => useGitStatus("/test/repo-a"));

		await vi.waitFor(() => {
			expect(gitStatusCallCount()).toBe(1);
		});

		act(() => {
			gitStatusCallback?.({ payload: { repo_path: "/test/repo-b" } });
		});

		await act(async () => {
			vi.advanceTimersByTime(300);
		});

		expect(gitStatusCallCount()).toBe(1);

		vi.useRealTimers();
	});
});
