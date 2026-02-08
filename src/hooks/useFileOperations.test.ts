import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useFileOperations } from "./useFileOperations";

const mockWriteTextFile = vi.fn().mockResolvedValue(undefined);
const mockMkdir = vi.fn().mockResolvedValue(undefined);
const mockRemove = vi.fn().mockResolvedValue(undefined);
const mockRename = vi.fn().mockResolvedValue(undefined);
const mockCopyFile = vi.fn().mockResolvedValue(undefined);
const mockExists = vi.fn().mockResolvedValue(true);
const mockReadDir = vi.fn().mockResolvedValue([]);

vi.mock("@tauri-apps/plugin-fs", () => ({
	writeTextFile: (...args: unknown[]) => mockWriteTextFile(...args),
	mkdir: (...args: unknown[]) => mockMkdir(...args),
	remove: (...args: unknown[]) => mockRemove(...args),
	rename: (...args: unknown[]) => mockRename(...args),
	copyFile: (...args: unknown[]) => mockCopyFile(...args),
	exists: (...args: unknown[]) => mockExists(...args),
	readDir: (...args: unknown[]) => mockReadDir(...args),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
	revealItemInDir: vi.fn().mockResolvedValue(undefined),
}));

describe("useFileOperations", () => {
	it("should create a file with empty content", async () => {
		const { result } = renderHook(() => useFileOperations());

		await act(async () => {
			await result.current.createFile("/test/new-file.ts");
		});

		expect(mockWriteTextFile).toHaveBeenCalledWith("/test/new-file.ts", "");
	});

	it("should create a folder recursively", async () => {
		const { result } = renderHook(() => useFileOperations());

		await act(async () => {
			await result.current.createFolder("/test/new-folder");
		});

		expect(mockMkdir).toHaveBeenCalledWith("/test/new-folder", {
			recursive: true,
		});
	});

	it("should delete item recursively", async () => {
		const { result } = renderHook(() => useFileOperations());

		await act(async () => {
			await result.current.deleteItem("/test/file.ts");
		});

		expect(mockRemove).toHaveBeenCalledWith("/test/file.ts", {
			recursive: true,
		});
	});

	it("should rename an item", async () => {
		const { result } = renderHook(() => useFileOperations());

		await act(async () => {
			await result.current.renameItem("/test/old.ts", "/test/new.ts");
		});

		expect(mockRename).toHaveBeenCalledWith("/test/old.ts", "/test/new.ts");
	});

	it("should check if file exists", async () => {
		const { result } = renderHook(() => useFileOperations());

		let exists = false;
		await act(async () => {
			exists = await result.current.fileExists("/test/file.ts");
		});

		expect(exists).toBe(true);
		expect(mockExists).toHaveBeenCalledWith("/test/file.ts");
	});

	describe("clipboard operations", () => {
		it("should set clipboard on cut", () => {
			const { result } = renderHook(() => useFileOperations());

			act(() => {
				result.current.cut("/test/file.ts", "file");
			});

			expect(result.current.clipboard).toEqual({
				operation: "cut",
				sourcePath: "/test/file.ts",
				type: "file",
			});
		});

		it("should set clipboard on copy", () => {
			const { result } = renderHook(() => useFileOperations());

			act(() => {
				result.current.copy("/test/file.ts", "file");
			});

			expect(result.current.clipboard).toEqual({
				operation: "copy",
				sourcePath: "/test/file.ts",
				type: "file",
			});
		});

		it("should rename on paste when clipboard is cut", async () => {
			const { result } = renderHook(() => useFileOperations());

			act(() => {
				result.current.cut("/test/file.ts", "file");
			});

			await act(async () => {
				await result.current.paste("/dest");
			});

			expect(mockRename).toHaveBeenCalledWith("/test/file.ts", "/dest/file.ts");
			expect(result.current.clipboard).toBeNull();
		});

		it("should copy file on paste when clipboard is copy", async () => {
			const { result } = renderHook(() => useFileOperations());

			act(() => {
				result.current.copy("/test/file.ts", "file");
			});

			await act(async () => {
				await result.current.paste("/dest");
			});

			expect(mockCopyFile).toHaveBeenCalledWith(
				"/test/file.ts",
				"/dest/file.ts",
			);
		});
	});
});
