import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke } from "@/lib/clientSocket";
import { useReviewFileView } from "./useReviewFileView";

vi.mock("@/lib/clientSocket", () => ({
	invokeClient: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useReviewFileView", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
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

	it("画像参照をWSで取得して画像データを表示する", async () => {
		mockInvoke
			.mockResolvedValueOnce({
				kind: "image",
				version: 3,
				stale: false,
				fileId: "image.png",
				path: "image.png",
				originalUrl: "blob?side=original",
				modifiedUrl: "blob?side=modified",
				mime: "image/png",
			})
			.mockResolvedValueOnce("data:image/png;base64,AQ==")
			.mockResolvedValueOnce("data:image/png;base64,Ag==");

		const { result } = renderHook(() =>
			useReviewFileView("/repo", "image.png", "head", "changes", 0, 3),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
			expect(result.current.imageDiff.modifiedUrl).toBe(
				"data:image/png;base64,Ag==",
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_blob", {
			reference: "blob?side=original",
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_review_blob", {
			reference: "blob?side=modified",
		});
		expect(result.current.originalContent).toBe("");
		expect(result.current.hunks).toBeNull();
		expect(result.current.imageDiff.originalUrl?.startsWith("data:")).toBe(
			true,
		);
		expect(result.current.error).toBeNull();
	});

	it("非画像binaryはblob取得せずメタデータだけで表示する", async () => {
		mockInvoke
			.mockResolvedValueOnce({
				kind: "binary",
				version: 3,
				stale: false,
				fileId: "a.bin",
				path: "a.bin",
				originalUrl: null,
				modifiedUrl: "blob?side=modified",
				originalSize: null,
				modifiedSize: 2,
			})
			.mockRejectedValueOnce(new Error("blob unavailable"));
		const { result } = renderHook(() =>
			useReviewFileView("/repo", "a.bin", "head", "changes", 0, 3),
		);
		await waitFor(() => expect(result.current.view?.kind).toBe("binary"));
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBeNull();
		expect(mockInvoke).toHaveBeenCalledTimes(1);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"get_review_blob",
			expect.anything(),
		);
	});

	it("破棄済み画像要求はblob取得を始めず最新画像だけを表示する", async () => {
		let finishOld!: (value: Awaited<ReturnType<typeof invoke>>) => void;
		mockInvoke.mockImplementation((command, args) => {
			if (command === "get_review_blob")
				return Promise.resolve("data:image/png;base64,AQ==");
			if (
				command === "get_review_file_view" &&
				(args as { input: { target: { value: string } } }).input.target
					.value === "old.png"
			)
				return new Promise((resolve) => {
					finishOld = resolve;
				});
			return Promise.resolve({
				kind: "image",
				version: 3,
				stale: false,
				fileId: "new.png",
				path: "new.png",
				originalUrl: null,
				modifiedUrl: "new-blob",
				mime: "image/png",
			});
		});
		const view = renderHook(
			({ path }) => useReviewFileView("/repo", path, "head", "changes", 0, 3),
			{ initialProps: { path: "old.png" } },
		);
		view.rerender({ path: "new.png" });
		await waitFor(() =>
			expect(view.result.current.imageDiff.modifiedUrl).toBe(
				"data:image/png;base64,AQ==",
			),
		);
		await act(async () =>
			finishOld({
				kind: "image",
				version: 2,
				stale: false,
				fileId: "old.png",
				path: "old.png",
				mime: "image/png",
				originalUrl: "old-original",
				modifiedUrl: "old-modified",
			}),
		);
		expect(
			mockInvoke.mock.calls.filter(
				([command]) => command === "get_review_blob",
			),
		).toEqual([["get_review_blob", { reference: "new-blob" }]]);
		expect(view.result.current.view?.path).toBe("new.png");
	});

	it.each([
		[
			{
				code: "REVIEW_FILE_VIEW_UNAVAILABLE",
				message: "coded review failure",
			},
			"coded review failure",
		],
		["plain review failure", "plain review failure"],
	])(
		"exposes backend message as an explicit error state",
		async (rejection, expected) => {
			mockInvoke.mockRejectedValue(rejection);

			const { result } = renderHook(() =>
				useReviewFileView("/repo", "missing.ts", "head", "changes", 0, 17),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
				expect(result.current.error).toBe(expected);
			});

			expect(result.current.view).toBeNull();
			expect(result.current.hunks).toBeNull();
		},
	);
});
