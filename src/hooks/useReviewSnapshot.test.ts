import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useReviewSnapshot } from "./useReviewSnapshot";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useReviewSnapshot", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("invokes get_review_snapshot and exposes trees without frontend rebuilding", async () => {
		mockInvoke.mockResolvedValue({
			version: 4,
			stale: false,
			loading: false,
			limited: false,
			base: "head",
			files: [{ fileId: "a.ts", path: "a.ts" }],
			status: [
				{
					path: "a.ts",
					index_status: "none",
					worktree_status: "modified",
				},
			],
			diffStats: [],
			tree: [],
			stagedTree: [],
			changesTree: [
				{
					id: "a.ts",
					name: "a.ts",
					path: "a.ts",
					node_type: "file",
					status: "modified",
					additions: 1,
					deletions: 0,
					children: [],
				},
			],
			stagedFileCount: 0,
			changesFileCount: 1,
		});

		const { result } = renderHook(() => useReviewSnapshot("/repo", "head", 0));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.version).toBe(4);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_snapshot", {
			input: { worktreePath: "/repo", base: "head" },
		});
		expect(result.current.changedFiles).toHaveLength(1);
		expect(result.current.changesTree[0].path).toBe("a.ts");
	});

	it("returns empty state when rootPath is null", () => {
		const { result } = renderHook(() =>
			useReviewSnapshot(null, "branch-base", 0),
		);

		expect(result.current.files).toEqual([]);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("resets accepted version when rootPath changes", async () => {
		mockInvoke.mockImplementation((_, args) => {
			const worktreePath = (args as { input: { worktreePath: string } }).input
				.worktreePath;
			return Promise.resolve({
				version: worktreePath === "/repo-a" ? 10 : 1,
				stale: false,
				loading: false,
				limited: false,
				base: "head",
				files: [{ fileId: `${worktreePath}/file.ts`, path: "file.ts" }],
				status: [],
				diffStats: [],
				tree: [],
				stagedTree: [],
				changesTree: [],
				stagedFileCount: 0,
				changesFileCount: 0,
			});
		});

		const { result, rerender } = renderHook(
			({ rootPath }) => useReviewSnapshot(rootPath, "head", 0),
			{ initialProps: { rootPath: "/repo-a" } },
		);

		await waitFor(() => {
			expect(result.current.version).toBe(10);
		});

		rerender({ rootPath: "/repo-b" });

		await waitFor(() => {
			expect(result.current.version).toBe(1);
			expect(result.current.files[0].fileId).toBe("/repo-b/file.ts");
		});
	});
});
