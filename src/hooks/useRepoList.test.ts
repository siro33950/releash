import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useRepoList } from "./useRepoList";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

type ListenCallback = (event: { payload: string[] }) => void;
let capturedListeners: Map<string, ListenCallback>;

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn((eventName: string, callback: ListenCallback) => {
		capturedListeners.set(eventName, callback);
		return Promise.resolve(() => {
			capturedListeners.delete(eventName);
		});
	}),
}));

describe("useRepoList", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		capturedListeners = new Map();
		mockInvoke.mockResolvedValue(undefined);
	});

	it("should call invoke('get_repo_paths') on mount and set repoPaths", async () => {
		mockInvoke.mockResolvedValueOnce(["/repo/a", "/repo/b"]);

		const { result } = renderHook(() => useRepoList());

		await vi.waitFor(() => {
			expect(result.current.repoPaths).toEqual(["/repo/a", "/repo/b"]);
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_repo_paths");
	});

	it("should not throw when invoke('get_repo_paths') fails", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("backend error"));

		const { result } = renderHook(() => useRepoList());

		await act(async () => {});

		expect(result.current.repoPaths).toEqual([]);
	});

	it("should update repoPaths when 'repo-paths-changed' event is received", async () => {
		mockInvoke.mockResolvedValueOnce(["/repo/a"]);

		const { result } = renderHook(() => useRepoList());

		await vi.waitFor(() => {
			expect(result.current.repoPaths).toEqual(["/repo/a"]);
		});

		act(() => {
			const listener = capturedListeners.get("repo-paths-changed");
			listener?.({ payload: ["/repo/a", "/repo/new"] });
		});

		expect(result.current.repoPaths).toEqual(["/repo/a", "/repo/new"]);
	});

	it("should call invoke('add_repo_path') when addRepo is called", async () => {
		mockInvoke.mockResolvedValueOnce([]);

		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.addRepo("/repo/new");
		});

		expect(mockInvoke).toHaveBeenCalledWith("add_repo_path", {
			path: "/repo/new",
		});
	});

	it("should call invoke('remove_repo_path') when removeRepo is called", async () => {
		mockInvoke.mockResolvedValueOnce(["/repo/a"]);

		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.removeRepo("/repo/a");
		});

		expect(mockInvoke).toHaveBeenCalledWith("remove_repo_path", {
			path: "/repo/a",
		});
	});

	it("should call invoke('add_repo_path') when initFromCwd is called", async () => {
		mockInvoke.mockResolvedValueOnce([]);

		const { result } = renderHook(() => useRepoList());
		await act(async () => {});

		act(() => {
			result.current.initFromCwd("/workspace/project");
		});

		expect(mockInvoke).toHaveBeenCalledWith("add_repo_path", {
			path: "/workspace/project",
		});
	});

	it("should cleanup event listener on unmount", async () => {
		mockInvoke.mockResolvedValueOnce([]);

		const { unmount } = renderHook(() => useRepoList());
		await act(async () => {});

		expect(capturedListeners.has("repo-paths-changed")).toBe(true);

		unmount();

		await vi.waitFor(() => {
			expect(capturedListeners.has("repo-paths-changed")).toBe(false);
		});
	});
});
