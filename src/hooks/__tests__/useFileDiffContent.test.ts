import { invoke } from "@tauri-apps/api/core";
import { readTextFile } from "@tauri-apps/plugin-fs";
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useFileDiffContent } from "../useFileDiffContent";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
	readTextFile: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);
const mockReadTextFile = vi.mocked(readTextFile);

describe("useFileDiffContent", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("returns empty strings when filePath is null", () => {
		const { result, unmount } = renderHook(() =>
			useFileDiffContent(null, "head", "changes", 0),
		);
		expect(result.current.originalContent).toBe("");
		expect(result.current.modifiedContent).toBe("");
		expect(result.current.loading).toBe(false);
		unmount();
	});

	it("fetches branch-base content (fetchBranchBase + fetchWorkingTree)", async () => {
		mockInvoke.mockResolvedValueOnce("base content");
		mockReadTextFile.mockResolvedValueOnce("working content");

		const { result, unmount } = renderHook(() =>
			useFileDiffContent("/repo/src/main.ts", "branch-base", "changes", 0),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalContent).toBe("base content");
		expect(result.current.modifiedContent).toBe("working content");
		expect(mockInvoke).toHaveBeenCalledWith("get_file_at_branch_base", {
			filePath: "/repo/src/main.ts",
		});
		expect(mockReadTextFile).toHaveBeenCalledWith("/repo/src/main.ts");
		unmount();
	});

	it("fetches head/staged content (fetchHead + fetchStaged)", async () => {
		mockInvoke
			.mockResolvedValueOnce("head content") // get_file_at_ref
			.mockResolvedValueOnce("staged content"); // get_staged_content

		const { result, unmount } = renderHook(() =>
			useFileDiffContent("/repo/src/main.ts", "head", "staged", 0),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalContent).toBe("head content");
		expect(result.current.modifiedContent).toBe("staged content");
		unmount();
	});

	it("fetches head/changes content (fetchStaged + fetchWorkingTree)", async () => {
		mockInvoke.mockResolvedValueOnce("staged content"); // get_staged_content
		mockReadTextFile.mockResolvedValueOnce("working content");

		const { result, unmount } = renderHook(() =>
			useFileDiffContent("/repo/src/main.ts", "head", "changes", 0),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalContent).toBe("staged content");
		expect(result.current.modifiedContent).toBe("working content");
		unmount();
	});

	it("handles fetch errors by returning empty strings and setting loading to false", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("fail"));
		mockReadTextFile.mockRejectedValueOnce(new Error("fail"));

		const { result, unmount } = renderHook(() =>
			useFileDiffContent("/repo/src/main.ts", "branch-base", "changes", 0),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalContent).toBe("");
		expect(result.current.modifiedContent).toBe("");
		unmount();
	});

	it("re-fetches when gitRefreshKey changes", async () => {
		mockInvoke.mockResolvedValue("content");
		mockReadTextFile.mockResolvedValue("working");

		const { result, rerender, unmount } = renderHook(
			({ refreshKey }) =>
				useFileDiffContent(
					"/repo/src/main.ts",
					"branch-base",
					"changes",
					refreshKey,
				),
			{ initialProps: { refreshKey: 0 } },
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		rerender({ refreshKey: 1 });

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Called twice: once for initial render, once for refreshKey change
		expect(mockInvoke).toHaveBeenCalledTimes(2);
		unmount();
	});
});
