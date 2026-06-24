import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useReviewFileView } from "./useReviewFileView";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useReviewFileView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("loads text diff content from get_review_file_view", async () => {
		mockInvoke.mockResolvedValue({
			kind: "textDiff",
			version: 17,
			stale: false,
			fileId: "src/main.ts",
			path: "src/main.ts",
			original: "old",
			modified: "new",
			source: "diff",
			hunks: [],
			changeGroups: [],
			limited: false,
			viewport: null,
			totalLines: 1,
		});

		const { result } = renderHook(() =>
			useReviewFileView("/repo", "src/main.ts", "head", "changes", 0, 17),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.originalContent).toBe("old");
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_file_view", {
			input: {
				worktreePath: "/repo",
				target: { by: "path", value: "src/main.ts" },
				section: "changes",
				base: "head",
				snapshotVersion: 17,
				viewport: null,
			},
		});
		expect(result.current.modifiedContent).toBe("new");
		expect(result.current.hunks).toEqual([]);
		expect(result.current.changeGroups).toEqual([]);
		expect(result.current.error).toBeNull();
	});

	it("passes image URL refs through without data URL conversion", async () => {
		mockInvoke.mockResolvedValue({
			kind: "image",
			version: 3,
			stale: false,
			fileId: "image.png",
			path: "image.png",
			originalUrl: "review-blob://localhost/blob?side=original",
			modifiedUrl: "review-blob://localhost/blob?side=modified",
			mime: "image/png",
		});

		const { result } = renderHook(() =>
			useReviewFileView("/repo", "image.png", "head", "changes", 0, 3),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.imageDiff.modifiedUrl).toBe(
				"review-blob://localhost/blob?side=modified",
			);
		});

		expect(result.current.originalContent).toBe("");
		expect(result.current.hunks).toBeNull();
		expect(result.current.imageDiff.originalUrl?.startsWith("data:")).toBe(
			false,
		);
		expect(result.current.error).toBeNull();
	});

	it("exposes command failures as an explicit error state", async () => {
		mockInvoke.mockRejectedValue("review target is not in snapshot");

		const { result } = renderHook(() =>
			useReviewFileView("/repo", "missing.ts", "head", "changes", 0, 17),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.error).toBe(
				"Failed to load diff: review target is not in snapshot",
			);
		});

		expect(result.current.view).toBeNull();
		expect(result.current.hunks).toBeNull();
	});
});
