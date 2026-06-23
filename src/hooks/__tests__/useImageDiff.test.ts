import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useImageDiff } from "../useImageDiff";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useImageDiff", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("returns null URLs when filePath is null", () => {
		const { result } = renderHook(() =>
			useImageDiff(null, "branch-base", "changes"),
		);
		expect(result.current.originalUrl).toBeNull();
		expect(result.current.modifiedUrl).toBeNull();
		expect(result.current.loading).toBe(false);
	});

	it("fetches original and modified through batched review image command", async () => {
		mockInvoke.mockResolvedValue({
			originalBase64: "iVBORw0KGgo=",
			modifiedBase64: "iVBORw0KGgo=",
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "branch-base", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_image_diff", {
			filePath: "/repo/image.png",
			diffBase: "branch-base",
			section: "changes",
		});

		expect(result.current.modifiedUrl).toMatch(/^data:image\/png;base64,/);
		expect(result.current.originalUrl).toBe(
			"data:image/png;base64,iVBORw0KGgo=",
		);
	});

	it("uses Staged→Working Tree when section is changes (HEAD mode)", async () => {
		mockInvoke.mockResolvedValue({
			originalBase64: "STAGED64",
			modifiedBase64: "AQID",
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "head", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_image_diff", {
			filePath: "/repo/image.png",
			diffBase: "head",
			section: "changes",
		});
	});

	it("uses HEAD→Staged when section is staged", async () => {
		mockInvoke.mockResolvedValue({
			originalBase64: "HEAD64",
			modifiedBase64: "STAGED64",
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "head", "staged"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_review_image_diff", {
			filePath: "/repo/image.png",
			diffBase: "head",
			section: "staged",
		});
	});

	it("sets originalUrl to null when invoke fails (new file)", async () => {
		mockInvoke.mockResolvedValue({
			originalBase64: null,
			modifiedBase64: "AQID",
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/new-image.png", "branch-base", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalUrl).toBeNull();
		expect(result.current.modifiedUrl).toMatch(/^data:image\/png;base64,/);
	});

	it("sets modifiedUrl to null when modified image is missing", async () => {
		mockInvoke.mockResolvedValue({
			originalBase64: "AQID",
			modifiedBase64: null,
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/deleted.png", "branch-base", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.modifiedUrl).toBeNull();
		expect(result.current.originalUrl).toBe("data:image/png;base64,AQID");
	});
});
