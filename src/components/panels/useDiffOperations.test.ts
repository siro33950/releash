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
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup("g:stage:0");
		});

		expect(mockInvoke).toHaveBeenCalledWith("git_stage_review_group", {
			input: {
				worktreePath: "/repo",
				path: "relative/path.ts",
				section: "changes",
				base: "head",
				groupId: "g:stage:0",
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
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleUnstageGroup("g:unstage:0");
		});

		expect(mockInvoke).toHaveBeenCalledWith("git_unstage_review_group", {
			input: {
				worktreePath: "/repo",
				path: "relative/path.ts",
				section: "staged",
				base: "head",
				groupId: "g:unstage:0",
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
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup("g:0");
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("does nothing when group id is missing", async () => {
		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "changes",
				base: "head",
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup("");
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("refreshes when the Rust command reports a stale review group target", async () => {
		const onGitChanged = vi.fn();
		mockInvoke.mockRejectedValue({
			code: "STALE_REVIEW_GROUP_TARGET",
			message: "review group target stale: g:old:0",
		});

		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "changes",
				base: "head",
				onGitChanged,
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup("g:old:0");
		});

		expect(onGitChanged).toHaveBeenCalled();
	});
});
