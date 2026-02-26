import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useGitDirWatcher } from "./useGitDirWatcher";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useGitDirWatcher", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should call start_git_dir_watching on mount", async () => {
		mockInvoke.mockResolvedValue(42);

		renderHook(() => useGitDirWatcher("/test/repo"));

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});
	});

	it("should call stop_watching on unmount", async () => {
		mockInvoke.mockResolvedValue(42);

		const { unmount } = renderHook(() => useGitDirWatcher("/test/repo"));

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});

		unmount();

		expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
			watcherId: 42,
		});
	});

	it("should not call start_git_dir_watching when repoPath is null", () => {
		renderHook(() => useGitDirWatcher(null));

		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should restart watcher when repoPath changes", async () => {
		mockInvoke
			.mockResolvedValueOnce(1)
			.mockResolvedValueOnce(undefined)
			.mockResolvedValueOnce(2);

		const { rerender } = renderHook(
			({ path }: { path: string }) => useGitDirWatcher(path),
			{ initialProps: { path: "/repo-a" } },
		);

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/repo-a",
			});
		});

		rerender({ path: "/repo-b" });

		await vi.waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
				watcherId: 1,
			});
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/repo-b",
			});
		});
	});

	it("should handle start_git_dir_watching error gracefully", async () => {
		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		mockInvoke.mockRejectedValue(new Error("not a git repo"));

		renderHook(() => useGitDirWatcher("/not/a/repo"));

		await vi.waitFor(() => {
			expect(consoleSpy).toHaveBeenCalledWith(
				"Failed to start git dir watching:",
				expect.any(Error),
			);
		});

		consoleSpy.mockRestore();
	});
});
