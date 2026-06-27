import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReviewSnapshot } from "@/types/review";
import { useReviewSnapshot } from "./useReviewSnapshot";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

function snapshot(overrides: Partial<ReviewSnapshot> = {}): ReviewSnapshot {
	return {
		version: 4,
		stale: false,
		loading: false,
		limited: false,
		base: "head",
		files: [],
		stagedFiles: [],
		changedFiles: [],
		diffStats: [],
		tree: [],
		stagedTree: [],
		changesTree: [],
		stagedFileCount: 0,
		changesFileCount: 0,
		...overrides,
	};
}

describe("useReviewSnapshot", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	it("invokes get_review_snapshot and exposes backend staged/changed files without rebuilding", async () => {
		mockInvoke.mockResolvedValue(
			snapshot({
				files: [
					{
						fileId: "a.ts",
						path: "a.ts",
						indexStatus: "none",
						worktreeStatus: "modified",
						additions: 1,
						deletions: 0,
					},
				],
				stagedFiles: [
					{
						path: "backend-staged.ts",
						index_status: "new",
						worktree_status: "none",
					},
				],
				changedFiles: [
					{
						path: "backend-changed.ts",
						index_status: "none",
						worktree_status: "modified",
					},
				],
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
				stagedFileCount: 1,
				changesFileCount: 1,
			}),
		);

		const { result } = renderHook(() => useReviewSnapshot("/repo", "head", 0));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.version).toBe(4);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_snapshot", {
			input: { worktreePath: "/repo", base: "head" },
		});
		expect(result.current.stagedFiles.map((entry) => entry.path)).toEqual([
			"backend-staged.ts",
		]);
		expect(result.current.changedFiles.map((entry) => entry.path)).toEqual([
			"backend-changed.ts",
		]);
		expect(result.current.changesTree[0].path).toBe("a.ts");
	});

	it("returns empty state when rootPath is null", () => {
		const { result } = renderHook(() =>
			useReviewSnapshot(null, "branch-base", 0),
		);

		expect(result.current.files).toEqual([]);
		expect(result.current.stagedFiles).toEqual([]);
		expect(result.current.changedFiles).toEqual([]);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("returns empty staged and changed files when fetching fails", async () => {
		mockInvoke.mockRejectedValue(new Error("boom"));

		const { result } = renderHook(() => useReviewSnapshot("/repo", "head", 0));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		expect(result.current.stagedFiles).toEqual([]);
		expect(result.current.changedFiles).toEqual([]);
		expect(result.current.version).toBe(0);
	});

	it("resets accepted version when rootPath changes", async () => {
		mockInvoke.mockImplementation((_, args) => {
			const worktreePath = (args as { input: { worktreePath: string } }).input
				.worktreePath;
			return Promise.resolve(
				snapshot({
					version: worktreePath === "/repo-a" ? 10 : 1,
					files: [
						{
							fileId: `${worktreePath}/file.ts`,
							path: "file.ts",
							indexStatus: "none",
							worktreeStatus: "modified",
							additions: 1,
							deletions: 0,
						},
					],
				}),
			);
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

	it("ignores snapshots older than the accepted version", async () => {
		mockInvoke
			.mockResolvedValueOnce(
				snapshot({
					version: 5,
					stagedFiles: [
						{
							path: "accepted.ts",
							index_status: "modified",
							worktree_status: "none",
						},
					],
				}),
			)
			.mockResolvedValueOnce(
				snapshot({
					version: 4,
					stagedFiles: [
						{
							path: "older.ts",
							index_status: "modified",
							worktree_status: "none",
						},
					],
				}),
			);

		const { result, rerender } = renderHook(
			({ refreshKey }) => useReviewSnapshot("/repo", "head", refreshKey),
			{ initialProps: { refreshKey: 0 } },
		);

		await waitFor(() => {
			expect(result.current.version).toBe(5);
		});

		rerender({ refreshKey: 1 });

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(2);
		});
		expect(result.current.version).toBe(5);
		expect(result.current.stagedFiles.map((entry) => entry.path)).toEqual([
			"accepted.ts",
		]);
	});

	it("accepts snapshot updates with the same version", async () => {
		mockInvoke
			.mockResolvedValueOnce(
				snapshot({
					version: 5,
					stagedFiles: [
						{
							path: "initial-staged.ts",
							index_status: "modified",
							worktree_status: "none",
						},
					],
					changedFiles: [
						{
							path: "initial-changed.ts",
							index_status: "none",
							worktree_status: "modified",
						},
					],
				}),
			)
			.mockResolvedValueOnce(
				snapshot({
					version: 5,
					stagedFiles: [
						{
							path: "same-version-staged.ts",
							index_status: "new",
							worktree_status: "none",
						},
					],
					changedFiles: [
						{
							path: "same-version-changed.ts",
							index_status: "none",
							worktree_status: "deleted",
						},
					],
				}),
			);

		const { result, rerender } = renderHook(
			({ refreshKey }) => useReviewSnapshot("/repo", "head", refreshKey),
			{ initialProps: { refreshKey: 0 } },
		);

		await waitFor(() => {
			expect(result.current.version).toBe(5);
			expect(result.current.stagedFiles.map((entry) => entry.path)).toEqual([
				"initial-staged.ts",
			]);
		});

		rerender({ refreshKey: 1 });

		await waitFor(() => {
			expect(result.current.version).toBe(5);
			expect(result.current.stagedFiles.map((entry) => entry.path)).toEqual([
				"same-version-staged.ts",
			]);
			expect(result.current.changedFiles.map((entry) => entry.path)).toEqual([
				"same-version-changed.ts",
			]);
		});
	});

	it("accepts snapshot updates with a newer version", async () => {
		mockInvoke
			.mockResolvedValueOnce(
				snapshot({
					version: 5,
					stagedFiles: [
						{
							path: "old-staged.ts",
							index_status: "modified",
							worktree_status: "none",
						},
					],
					changedFiles: [
						{
							path: "old-changed.ts",
							index_status: "none",
							worktree_status: "modified",
						},
					],
				}),
			)
			.mockResolvedValueOnce(
				snapshot({
					version: 6,
					stagedFiles: [
						{
							path: "newer-staged.ts",
							index_status: "deleted",
							worktree_status: "none",
						},
					],
					changedFiles: [
						{
							path: "newer-changed.ts",
							index_status: "none",
							worktree_status: "new",
						},
					],
				}),
			);

		const { result, rerender } = renderHook(
			({ refreshKey }) => useReviewSnapshot("/repo", "head", refreshKey),
			{ initialProps: { refreshKey: 0 } },
		);

		await waitFor(() => {
			expect(result.current.version).toBe(5);
			expect(result.current.changedFiles.map((entry) => entry.path)).toEqual([
				"old-changed.ts",
			]);
		});

		rerender({ refreshKey: 1 });

		await waitFor(() => {
			expect(result.current.version).toBe(6);
			expect(result.current.stagedFiles.map((entry) => entry.path)).toEqual([
				"newer-staged.ts",
			]);
			expect(result.current.changedFiles.map((entry) => entry.path)).toEqual([
				"newer-changed.ts",
			]);
		});
	});
});
