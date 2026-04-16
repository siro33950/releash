import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useBranchDiffFiles } from "./useBranchDiffFiles";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useBranchDiffFiles", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	it("should return empty files when rootPath is null", async () => {
		const { result } = renderHook(() => useBranchDiffFiles(null, true, "main"));
		expect(result.current.files).toEqual([]);
		expect(result.current.error).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should return empty files when disabled", async () => {
		const { result } = renderHook(() =>
			useBranchDiffFiles("/repo", false, "main"),
		);
		expect(result.current.files).toEqual([]);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should fetch changed files when enabled", async () => {
		mockInvoke.mockResolvedValue({
			base_branch: "main",
			changed_files: [
				{
					path: "src/a.ts",
					old_path: null,
					status: "modified",
					binary: false,
					stats: { additions: 5, deletions: 2 },
				},
			],
			stats: { additions: 5, deletions: 2 },
		});

		const { result } = renderHook(() =>
			useBranchDiffFiles("/repo", true, "main"),
		);

		await waitFor(() => {
			expect(result.current.files).toHaveLength(1);
		});
		expect(result.current.files[0].path).toBe("src/a.ts");
		expect(mockInvoke).toHaveBeenCalledWith("get_branch_diff_summary", {
			repoPath: "/repo",
			baseBranch: "main",
		});
	});

	it("should set error when invoke rejects", async () => {
		mockInvoke.mockRejectedValue(new Error("base branch not found"));

		const { result } = renderHook(() =>
			useBranchDiffFiles("/repo", true, "main"),
		);

		await waitFor(() => {
			expect(result.current.error).toBe("base branch not found");
		});
		expect(result.current.files).toEqual([]);
	});

	it("should refetch via refresh()", async () => {
		mockInvoke.mockResolvedValue({
			base_branch: "main",
			changed_files: [],
			stats: { additions: 0, deletions: 0 },
		});

		const { result } = renderHook(() =>
			useBranchDiffFiles("/repo", true, "main"),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		await act(async () => {
			await result.current.refresh();
		});

		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("should pass null baseBranch through", async () => {
		mockInvoke.mockResolvedValue({
			base_branch: "HEAD",
			changed_files: [],
			stats: { additions: 0, deletions: 0 },
		});

		renderHook(() => useBranchDiffFiles("/repo", true, null));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_branch_diff_summary", {
				repoPath: "/repo",
				baseBranch: null,
			});
		});
	});
});
