import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CommitInfo } from "@/types/git";
import { useGitLog } from "./useGitLog";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useGitLog", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should return empty commits when rootPath is null", () => {
		const { result } = renderHook(() => useGitLog(null));

		expect(result.current.commits).toEqual([]);
		expect(result.current.loading).toBe(false);
	});

	it("should fetch commit log", async () => {
		const mockCommits: CommitInfo[] = [
			{
				hash: "abc1234567890",
				short_hash: "abc1234",
				message: "latest commit",
				author_name: "Test",
				author_email: "test@test.com",
				timestamp: 1700000000,
			},
			{
				hash: "def5678901234",
				short_hash: "def5678",
				message: "initial commit",
				author_name: "Test",
				author_email: "test@test.com",
				timestamp: 1699000000,
			},
		];
		mockInvoke.mockResolvedValue(mockCommits);

		const { result } = renderHook(() => useGitLog("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.commits).toHaveLength(2);
		});

		expect(result.current.commits[0].message).toBe("latest commit");
		expect(mockInvoke).toHaveBeenCalledWith("get_git_log", {
			repoPath: "/test/repo",
			limit: 50,
		});
	});

	it("should pass custom limit", async () => {
		mockInvoke.mockResolvedValue([]);

		renderHook(() => useGitLog("/test/repo", 10));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_git_log", {
				repoPath: "/test/repo",
				limit: 10,
			});
		});
	});

	it("should handle error gracefully", async () => {
		mockInvoke.mockRejectedValue(new Error("failed"));

		const { result } = renderHook(() => useGitLog("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.commits).toEqual([]);
	});
});
