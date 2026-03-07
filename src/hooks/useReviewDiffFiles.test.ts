import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useReviewDiffFiles } from "./useReviewDiffFiles";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(vi.fn()),
}));

describe("useReviewDiffFiles", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should not invoke when enabled is false", () => {
		renderHook(() => useReviewDiffFiles("/test/repo", false, "main"));
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should not invoke when rootPath is null", () => {
		renderHook(() => useReviewDiffFiles(null, true, "main"));
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should fetch files when enabled", async () => {
		mockInvoke.mockResolvedValue({
			base_ref: "main",
			changed_files: [
				{
					path: "src/app.tsx",
					old_path: null,
					status: "modified",
					binary: false,
					stats: { additions: 5, deletions: 2 },
					hunks: [],
					truncated: false,
				},
			],
			stats: { files_changed: 1, insertions: 5, deletions: 2 },
		});

		const { result } = renderHook(() =>
			useReviewDiffFiles("/test/repo", true, "main"),
		);

		await waitFor(() => {
			expect(result.current.files).toHaveLength(1);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_diff_summary", {
			repoPath: "/test/repo",
			baseBranch: "main",
		});
		expect(result.current.files[0].path).toBe("src/app.tsx");
		expect(result.current.error).toBeNull();
	});

	it("should return empty array and error message on error", async () => {
		mockInvoke.mockRejectedValue(new Error("fail"));

		const { result } = renderHook(() =>
			useReviewDiffFiles("/test/repo", true, "main"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.files).toEqual([]);
		expect(result.current.error).toBe("Error: fail");
	});

	it("should pass undefined baseBranch when null", async () => {
		mockInvoke.mockResolvedValue({
			base_ref: "HEAD",
			changed_files: [],
			stats: { files_changed: 0, insertions: 0, deletions: 0 },
		});

		renderHook(() => useReviewDiffFiles("/test/repo", true, null));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("get_review_diff_summary", {
				repoPath: "/test/repo",
				baseBranch: undefined,
			});
		});
	});
});
