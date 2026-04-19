import { invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useImageDiff } from "../useImageDiff";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
	readFile: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);
const mockReadFile = vi.mocked(readFile);

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

	it("fetches modified from readFile and original from get_binary_file_at_branch_base", async () => {
		const pngBytes = new Uint8Array([137, 80, 78, 71]);
		mockReadFile.mockResolvedValue(pngBytes);
		mockInvoke.mockResolvedValue("iVBORw0KGgo=");

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "branch-base", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(mockReadFile).toHaveBeenCalledWith("/repo/image.png");
		expect(mockInvoke).toHaveBeenCalledWith("get_binary_file_at_branch_base", {
			filePath: "/repo/image.png",
		});

		expect(result.current.modifiedUrl).toMatch(/^data:image\/png;base64,/);
		expect(result.current.originalUrl).toBe(
			"data:image/png;base64,iVBORw0KGgo=",
		);
	});

	it("uses Staged→Working Tree when section is changes (HEAD mode)", async () => {
		const pngBytes = new Uint8Array([1, 2, 3]);
		mockReadFile.mockResolvedValue(pngBytes);
		mockInvoke.mockResolvedValue("AQID");

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "head", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// original = get_binary_staged_content
		expect(mockInvoke).toHaveBeenCalledWith("get_binary_staged_content", {
			filePath: "/repo/image.png",
		});
		// modified = readFile
		expect(mockReadFile).toHaveBeenCalledWith("/repo/image.png");
	});

	it("uses HEAD→Staged when section is staged", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "get_binary_file_at_ref") return Promise.resolve("HEAD64");
			if (cmd === "get_binary_staged_content")
				return Promise.resolve("STAGED64");
			return Promise.resolve("");
		});

		const { result } = renderHook(() =>
			useImageDiff("/repo/image.png", "head", "staged"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// original = HEAD
		expect(mockInvoke).toHaveBeenCalledWith("get_binary_file_at_ref", {
			filePath: "/repo/image.png",
			gitRef: "HEAD",
		});
		// modified = staged
		expect(mockInvoke).toHaveBeenCalledWith("get_binary_staged_content", {
			filePath: "/repo/image.png",
		});
	});

	it("sets originalUrl to null when invoke fails (new file)", async () => {
		const pngBytes = new Uint8Array([1, 2, 3]);
		mockReadFile.mockResolvedValue(pngBytes);
		mockInvoke.mockRejectedValue(new Error("not found"));

		const { result } = renderHook(() =>
			useImageDiff("/repo/new-image.png", "branch-base", "changes"),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.originalUrl).toBeNull();
		expect(result.current.modifiedUrl).toMatch(/^data:image\/png;base64,/);
	});

	it("sets modifiedUrl to null when readFile fails (deleted file)", async () => {
		mockReadFile.mockRejectedValue(new Error("file not found"));
		mockInvoke.mockResolvedValue("AQID");

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
