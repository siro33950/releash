import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke } from "@/lib/client";
import { useDiffOperations } from "./useDiffOperations";

vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useDiffOperations", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("handleStageGroup delegates to the Rust review group command", async () => {
		mockInvoke.mockResolvedValue(undefined);

		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "changes",
				base: "head",
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
	});

	it("handleUnstageGroup delegates to the Rust review group command", async () => {
		mockInvoke.mockResolvedValue(undefined);

		const { result } = renderHook(() =>
			useDiffOperations({
				rootPath: "/repo",
				filePath: "relative/path.ts",
				section: "staged",
				base: "head",
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

	it("staleなreview groupの拒否を表示し次の版の配信を待つ", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		const error = vi.spyOn(console, "error").mockImplementation(() => {});
		const notice = vi.fn((_event: Event) => {});
		window.addEventListener("releash-client-error", notice);
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
			}),
		);

		await act(async () => {
			await result.current.handleStageGroup("g:old:0");
		});

		expect(warn).toHaveBeenCalledOnce();
		expect(error).not.toHaveBeenCalled();
		expect(notice).toHaveBeenCalledOnce();
		expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
			"review group target stale: g:old:0",
		);
		window.removeEventListener("releash-client-error", notice);
		warn.mockRestore();
		error.mockRestore();
	});
});
