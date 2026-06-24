import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDiffOperations } from "./useDiffOperations";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useDiffOperations", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("handleStageGroup delegates to the Rust review group command", async () => {
		const onGitChanged = vi.fn();
		mockInvoke.mockResolvedValue(undefined);

		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "changes",
				base: "head",
				snapshotVersion: 17,
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(2);
		});

		expect(mockInvoke).toHaveBeenCalledWith("git_stage_review_group", {
			input: {
				worktreePath: "/repo",
				path: "relative/path.ts",
				section: "changes",
				base: "head",
				groupIndex: 2,
				snapshotVersion: 17,
			},
		});
		expect(onGitChanged).toHaveBeenCalled();
	});

	it("handleUnstageGroup delegates to the Rust review group command", async () => {
		const onGitChanged = vi.fn();
		mockInvoke.mockResolvedValue(undefined);

		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "staged",
				base: "head",
				snapshotVersion: 23,
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleUnstageGroup(0);
		});

		expect(mockInvoke).toHaveBeenCalledWith("git_unstage_review_group", {
			input: {
				worktreePath: "/repo",
				path: "relative/path.ts",
				section: "staged",
				base: "head",
				groupIndex: 0,
				snapshotVersion: 23,
			},
		});
		expect(onGitChanged).toHaveBeenCalled();
	});

	it("does nothing when target identifiers are missing", async () => {
		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: null,
				section: "changes",
				base: "head",
				snapshotVersion: 1,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(0);
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("does nothing when snapshot version is missing", async () => {
		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "changes",
				base: "head",
				snapshotVersion: null,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup(0);
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});
});
