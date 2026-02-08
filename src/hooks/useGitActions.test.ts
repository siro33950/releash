import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useGitActions } from "./useGitActions";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useGitActions", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("stage should invoke git_stage with correct args", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useGitActions());

		await result.current.stage("/repo", ["file.txt"]);
		expect(mockInvoke).toHaveBeenCalledWith("git_stage", {
			repoPath: "/repo",
			paths: ["file.txt"],
		});
	});

	it("stage with empty paths should stage all", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useGitActions());

		await result.current.stage("/repo", []);
		expect(mockInvoke).toHaveBeenCalledWith("git_stage", {
			repoPath: "/repo",
			paths: [],
		});
	});

	it("unstage should invoke git_unstage with correct args", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useGitActions());

		await result.current.unstage("/repo", ["file.txt"]);
		expect(mockInvoke).toHaveBeenCalledWith("git_unstage", {
			repoPath: "/repo",
			paths: ["file.txt"],
		});
	});

	it("commit should invoke git_commit and return hash", async () => {
		mockInvoke.mockResolvedValue("abc123");
		const { result } = renderHook(() => useGitActions());

		const hash = await result.current.commit("/repo", "test message");
		expect(mockInvoke).toHaveBeenCalledWith("git_commit", {
			repoPath: "/repo",
			message: "test message",
		});
		expect(hash).toBe("abc123");
	});

	it("push should invoke git_push and return output", async () => {
		mockInvoke.mockResolvedValue("Everything up-to-date");
		const { result } = renderHook(() => useGitActions());

		const output = await result.current.push("/repo");
		expect(mockInvoke).toHaveBeenCalledWith("git_push", {
			repoPath: "/repo",
		});
		expect(output).toBe("Everything up-to-date");
	});

	it("createBranch should invoke git_create_branch", async () => {
		mockInvoke.mockResolvedValue(undefined);
		const { result } = renderHook(() => useGitActions());

		await result.current.createBranch("/repo", "feature");
		expect(mockInvoke).toHaveBeenCalledWith("git_create_branch", {
			repoPath: "/repo",
			branchName: "feature",
		});
	});

	it("should propagate errors from invoke", async () => {
		mockInvoke.mockRejectedValue(new Error("git error"));
		const { result } = renderHook(() => useGitActions());

		await expect(result.current.stage("/repo", [])).rejects.toThrow(
			"git error",
		);
	});
});
