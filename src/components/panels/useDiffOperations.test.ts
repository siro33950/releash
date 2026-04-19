import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useDiffOperations } from "./useDiffOperations";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useDiffOperations", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("handleStageGroup calls invoke chain and onStageHunk", async () => {
		const onStageHunk = vi.fn().mockResolvedValue(undefined);
		const onGitChanged = vi.fn();

		mockInvoke
			.mockResolvedValueOnce("relative/path.ts") // get_relative_path
			.mockResolvedValueOnce({
				// compute_diff_hunks
				hunks: [{ index: 0 }],
				changeGroups: [{ groupIndex: 0, hunkIndex: 0 }],
			})
			.mockResolvedValueOnce("patch-content"); // generate_group_patch

		const { result } = renderHook(() =>
			useDiffOperations({
				filePath: "/repo/relative/path.ts",
				rootPath: "/repo",
				originalContent: "original",
				modifiedContent: "modified",
				onStageHunk,
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(0);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_relative_path", {
			rootPath: "/repo",
			filePath: "/repo/relative/path.ts",
		});
		expect(mockInvoke).toHaveBeenCalledWith("compute_diff_hunks", {
			original: "original",
			modified: "modified",
			filePath: "relative/path.ts",
		});
		expect(mockInvoke).toHaveBeenCalledWith("generate_group_patch", {
			filePath: "relative/path.ts",
			hunk: { index: 0 },
			group: { groupIndex: 0, hunkIndex: 0 },
		});
		expect(onStageHunk).toHaveBeenCalledWith("/repo", "patch-content");
		expect(onGitChanged).toHaveBeenCalled();
	});

	it("handleUnstageGroup calls onUnstageHunk", async () => {
		const onUnstageHunk = vi.fn().mockResolvedValue(undefined);
		const onGitChanged = vi.fn();

		mockInvoke
			.mockResolvedValueOnce("relative/path.ts")
			.mockResolvedValueOnce({
				hunks: [{ index: 0 }],
				changeGroups: [{ groupIndex: 0, hunkIndex: 0 }],
			})
			.mockResolvedValueOnce("patch-content");

		const { result } = renderHook(() =>
			useDiffOperations({
				filePath: "/repo/relative/path.ts",
				rootPath: "/repo",
				originalContent: "original",
				modifiedContent: "modified",
				onUnstageHunk,
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleUnstageGroup(0);
		});

		expect(onUnstageHunk).toHaveBeenCalledWith("/repo", "patch-content");
		expect(onGitChanged).toHaveBeenCalled();
	});

	it("does nothing when rootPath is null", async () => {
		const onStageHunk = vi.fn();

		const { result } = renderHook(() =>
			useDiffOperations({
				filePath: "/repo/file.ts",
				rootPath: null,
				originalContent: "original",
				modifiedContent: "modified",
				onStageHunk,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(0);
		});

		expect(mockInvoke).not.toHaveBeenCalled();
		expect(onStageHunk).not.toHaveBeenCalled();
	});

	it("does nothing when group is not found", async () => {
		const onStageHunk = vi.fn();
		const onGitChanged = vi.fn();

		mockInvoke.mockResolvedValueOnce("relative/path.ts").mockResolvedValueOnce({
			hunks: [{ index: 0 }],
			changeGroups: [{ groupIndex: 5, hunkIndex: 0 }],
		});

		const { result } = renderHook(() =>
			useDiffOperations({
				filePath: "/repo/relative/path.ts",
				rootPath: "/repo",
				originalContent: "original",
				modifiedContent: "modified",
				onStageHunk,
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(99);
		});

		expect(onStageHunk).not.toHaveBeenCalled();
		expect(onGitChanged).not.toHaveBeenCalled();
	});
});
