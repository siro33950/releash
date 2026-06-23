import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useFileDiffContent } from "./useFileDiffContent";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useFileDiffContent", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	describe("Changes section (Staged → Working Tree)", () => {
		it("returns staged as original and working tree as modified", async () => {
			mockInvoke.mockResolvedValue({
				original: "staged content",
				modified: "working tree content",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("file.ts", "head", "changes", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("staged content");
			expect(result.current.modifiedContent).toBe("working tree content");
			expect(mockInvoke).toHaveBeenCalledWith("get_review_text_diff", {
				filePath: "file.ts",
				diffBase: "head",
				section: "changes",
			});
		});

		it("falls back to HEAD for original when both staged and working tree are empty (deleted file)", async () => {
			mockInvoke.mockResolvedValue({
				original: "head content",
				modified: "",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("deleted.ts", "head", "changes", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("head content");
			expect(result.current.modifiedContent).toBe("");
		});

		it("does not fall back to HEAD when both staged and working tree are genuinely empty (not deleted)", async () => {
			mockInvoke.mockResolvedValue({
				original: "",
				modified: "",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("empty-file.ts", "head", "changes", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("");
			expect(result.current.modifiedContent).toBe("");
		});

		it("does not fall back to HEAD when staged has content but working tree is empty", async () => {
			mockInvoke.mockResolvedValue({
				original: "staged content",
				modified: "",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("file.ts", "head", "changes", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("staged content");
			expect(result.current.modifiedContent).toBe("");
		});
	});

	describe("Staged Changes section (HEAD → Staged)", () => {
		it("returns HEAD as original and staged as modified", async () => {
			mockInvoke.mockResolvedValue({
				original: "head content",
				modified: "staged content",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("file.ts", "head", "staged", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("head content");
			expect(result.current.modifiedContent).toBe("staged content");
		});

		it("returns empty modified when file is staged for deletion", async () => {
			mockInvoke.mockResolvedValue({
				original: "head content",
				modified: "",
			});

			const { result } = renderHook(() =>
				useFileDiffContent("deleted.ts", "head", "staged", 0),
			);

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
			});
			expect(result.current.originalContent).toBe("head content");
			expect(result.current.modifiedContent).toBe("");
		});
	});

	describe("null filePath", () => {
		it("returns empty content and no loading", () => {
			const { result } = renderHook(() =>
				useFileDiffContent(null, "head", "changes", 0),
			);

			expect(result.current.originalContent).toBe("");
			expect(result.current.modifiedContent).toBe("");
			expect(result.current.loading).toBe(false);
		});
	});
});
