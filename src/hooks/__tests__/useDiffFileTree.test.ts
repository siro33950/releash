import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GitFileStatus } from "@/types/git";
import type { BranchDiffChangedFile } from "../useBranchDiffFiles";
import type { DiffTreeNode } from "../useDiffFileTree";
import { useDiffFileTree } from "../useDiffFileTree";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

const EMPTY_BRANCH_FILES: BranchDiffChangedFile[] = [];
const EMPTY_STAGED: GitFileStatus[] = [];
const EMPTY_CHANGED: GitFileStatus[] = [];

describe("useDiffFileTree", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("returns empty trees when no files", () => {
		const { result, unmount } = renderHook(() =>
			useDiffFileTree(
				"branch-base",
				EMPTY_BRANCH_FILES,
				EMPTY_STAGED,
				EMPTY_CHANGED,
			),
		);
		expect(result.current.branchBaseTree).toEqual([]);
		expect(result.current.branchBaseFileCount).toBe(0);
		expect(result.current.stagedTree).toEqual([]);
		expect(result.current.changesTree).toEqual([]);
		expect(result.current.stagedFileCount).toBe(0);
		expect(result.current.changesFileCount).toBe(0);
		expect(result.current.loading).toBe(false);
		unmount();
	});

	it("builds tree from branch-base diff files", async () => {
		const mockTree: DiffTreeNode[] = [
			{
				id: "folder:src",
				name: "src",
				path: "src",
				node_type: "folder",
				status: null,
				additions: null,
				deletions: null,
				children: [
					{
						id: "file:src/main.ts",
						name: "main.ts",
						path: "src/main.ts",
						node_type: "file",
						status: "modified",
						additions: 1,
						deletions: 0,
						children: [],
					},
				],
			},
		];

		mockInvoke.mockResolvedValueOnce(mockTree);

		const branchFiles: BranchDiffChangedFile[] = [
			{
				path: "src/main.ts",
				status: "modified",
				old_path: null,
				binary: false,
				stats: { additions: 1, deletions: 0 },
			},
		];

		const { result, unmount } = renderHook(() =>
			useDiffFileTree("branch-base", branchFiles, EMPTY_STAGED, EMPTY_CHANGED),
		);

		await waitFor(() => {
			expect(result.current.branchBaseTree).toEqual(mockTree);
		});

		expect(mockInvoke).toHaveBeenCalledWith("build_diff_file_tree", {
			entries: [
				{ path: "src/main.ts", status: "modified", additions: 1, deletions: 0 },
			],
		});
		expect(result.current.branchBaseFileCount).toBe(1);
		unmount();
	});

	it("head mode builds two separate trees (staged + changes)", async () => {
		const mockStagedTree: DiffTreeNode[] = [
			{
				id: "file:a.ts",
				name: "a.ts",
				path: "a.ts",
				node_type: "file",
				status: "modified",
				additions: 10,
				deletions: 3,
				children: [],
			},
		];
		const mockChangesTree: DiffTreeNode[] = [
			{
				id: "file:b.ts",
				name: "b.ts",
				path: "b.ts",
				node_type: "file",
				status: "new",
				additions: 1,
				deletions: 0,
				children: [],
			},
		];

		mockInvoke.mockResolvedValueOnce({
			version: 7,
			stale: false,
			loading: false,
			limited: false,
			combined_tree: [...mockStagedTree, ...mockChangesTree],
			staged_tree: mockStagedTree,
			changes_tree: mockChangesTree,
			staged_file_count: 1,
			changes_file_count: 1,
		});

		const staged: GitFileStatus[] = [
			{ path: "a.ts", index_status: "modified", worktree_status: "none" },
		];
		const changed: GitFileStatus[] = [
			{ path: "b.ts", index_status: "none", worktree_status: "new" },
		];

		const { result, unmount } = renderHook(() =>
			useDiffFileTree("head", EMPTY_BRANCH_FILES, staged, changed, "/repo"),
		);

		await waitFor(() => {
			expect(result.current.stagedTree).toEqual(mockStagedTree);
			expect(result.current.changesTree).toEqual(mockChangesTree);
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"get_head_diff_file_tree_snapshot",
			{
				repoPath: "/repo",
			},
		);
		expect(result.current.stagedFileCount).toBe(1);
		expect(result.current.changesFileCount).toBe(1);
		unmount();
	});

	it("head mode with only changedFiles builds changes tree", async () => {
		const mockTree: DiffTreeNode[] = [
			{
				id: "file:README.md",
				name: "README.md",
				path: "README.md",
				node_type: "file",
				status: "modified",
				additions: 0,
				deletions: 0,
				children: [],
			},
		];

		mockInvoke.mockResolvedValueOnce({
			version: 1,
			stale: false,
			loading: false,
			limited: false,
			combined_tree: mockTree,
			staged_tree: [],
			changes_tree: mockTree,
			staged_file_count: 0,
			changes_file_count: 1,
		});

		const changed: GitFileStatus[] = [
			{ path: "README.md", index_status: "none", worktree_status: "modified" },
		];

		const { result, unmount } = renderHook(() =>
			useDiffFileTree(
				"head",
				EMPTY_BRANCH_FILES,
				EMPTY_STAGED,
				changed,
				"/repo",
			),
		);

		await waitFor(() => {
			expect(result.current.changesTree).toEqual(mockTree);
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"get_head_diff_file_tree_snapshot",
			{
				repoPath: "/repo",
			},
		);
		expect(result.current.stagedFileCount).toBe(0);
		expect(result.current.changesFileCount).toBe(1);
		unmount();
	});

	it("head mode with only stagedFiles builds staged tree", async () => {
		const mockStagedTree: DiffTreeNode[] = [
			{
				id: "file:a.ts",
				name: "a.ts",
				path: "a.ts",
				node_type: "file",
				status: "modified",
				additions: 3,
				deletions: 1,
				children: [],
			},
		];

		mockInvoke.mockResolvedValueOnce({
			version: 2,
			stale: false,
			loading: false,
			limited: false,
			combined_tree: mockStagedTree,
			staged_tree: mockStagedTree,
			changes_tree: [],
			staged_file_count: 1,
			changes_file_count: 0,
		});

		const staged: GitFileStatus[] = [
			{ path: "a.ts", index_status: "modified", worktree_status: "none" },
		];

		const { result, unmount } = renderHook(() =>
			useDiffFileTree(
				"head",
				EMPTY_BRANCH_FILES,
				staged,
				EMPTY_CHANGED,
				"/repo",
			),
		);

		await waitFor(() => {
			expect(result.current.stagedTree).toEqual(mockStagedTree);
		});

		expect(result.current.stagedFileCount).toBe(1);
		expect(result.current.changesFileCount).toBe(0);
		unmount();
	});

	it("head mode refetches when status version changes without status list changes", async () => {
		const firstTree: DiffTreeNode[] = [
			{
				id: "file:a.ts",
				name: "a.ts",
				path: "a.ts",
				node_type: "file",
				status: "modified",
				additions: 1,
				deletions: 0,
				children: [],
			},
		];
		const secondTree: DiffTreeNode[] = [
			{
				id: "file:a.ts",
				name: "a.ts",
				path: "a.ts",
				node_type: "file",
				status: "modified",
				additions: 2,
				deletions: 1,
				children: [],
			},
		];
		mockInvoke
			.mockResolvedValueOnce({
				version: 1,
				stale: false,
				loading: false,
				limited: false,
				combined_tree: firstTree,
				staged_tree: [],
				changes_tree: firstTree,
				staged_file_count: 0,
				changes_file_count: 1,
			})
			.mockResolvedValueOnce({
				version: 2,
				stale: false,
				loading: false,
				limited: false,
				combined_tree: secondTree,
				staged_tree: [],
				changes_tree: secondTree,
				staged_file_count: 0,
				changes_file_count: 1,
			});

		const changed: GitFileStatus[] = [
			{ path: "a.ts", index_status: "none", worktree_status: "modified" },
		];

		const { result, rerender, unmount } = renderHook(
			({ statusVersion }: { statusVersion: number }) =>
				useDiffFileTree(
					"head",
					EMPTY_BRANCH_FILES,
					EMPTY_STAGED,
					changed,
					"/repo",
					statusVersion,
				),
			{ initialProps: { statusVersion: 1 } },
		);

		await waitFor(() => {
			expect(result.current.changesTree).toEqual(firstTree);
		});

		rerender({ statusVersion: 2 });

		await waitFor(() => {
			expect(result.current.changesTree).toEqual(secondTree);
		});

		expect(mockInvoke).toHaveBeenCalledTimes(2);
		expect(mockInvoke).toHaveBeenNthCalledWith(
			2,
			"get_head_diff_file_tree_snapshot",
			{
				repoPath: "/repo",
			},
		);
		unmount();
	});

	it("head mode returns empty trees when no files", () => {
		const { result, unmount } = renderHook(() =>
			useDiffFileTree(
				"head",
				EMPTY_BRANCH_FILES,
				EMPTY_STAGED,
				EMPTY_CHANGED,
				"/repo",
			),
		);

		expect(result.current.stagedTree).toEqual([]);
		expect(result.current.changesTree).toEqual([]);
		expect(result.current.stagedFileCount).toBe(0);
		expect(result.current.changesFileCount).toBe(0);
		unmount();
	});
});
