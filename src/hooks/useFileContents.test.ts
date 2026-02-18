import { readTextFile } from "@tauri-apps/plugin-fs";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { detectEol, useFileContents } from "./useFileContents";

vi.mock("@tauri-apps/plugin-fs", () => ({
	readTextFile: vi.fn().mockResolvedValue("file content"),
	writeTextFile: vi.fn().mockResolvedValue(undefined),
}));

describe("detectEol", () => {
	it("should return LF for content without CRLF", () => {
		expect(detectEol("line1\nline2\nline3")).toBe("LF");
	});

	it("should return CRLF for content with CRLF", () => {
		expect(detectEol("line1\r\nline2\r\nline3")).toBe("CRLF");
	});

	it("should return LF for empty content", () => {
		expect(detectEol("")).toBe("LF");
	});
});

describe("useFileContents", () => {
	describe("openFile", () => {
		it("should open a file and add to files list", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			expect(result.current.files).toHaveLength(1);
			expect(result.current.files[0].path).toBe("/test/file.ts");
			expect(result.current.files[0].content).toBe("file content");
		});

		it("should not duplicate file when opening same path", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			expect(result.current.files).toHaveLength(1);
		});
	});

	describe("getFileContent", () => {
		it("should return file content by path", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			const file = result.current.getFileContent("/test/file.ts");
			expect(file?.content).toBe("file content");
		});

		it("should return undefined for unknown path", () => {
			const { result } = renderHook(() => useFileContents());
			expect(result.current.getFileContent("/unknown")).toBeUndefined();
		});
	});

	describe("eol detection", () => {
		it("should set eol to LF when file has LF line endings", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\nline2");
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/lf.ts");
			});

			expect(result.current.files[0].eol).toBe("LF");
		});

		it("should set eol to CRLF when file has CRLF line endings", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\r\nline2");
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/crlf.ts");
			});

			expect(result.current.files[0].eol).toBe("CRLF");
		});

		it("should update eol on reloadFileIfClean", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\nline2");
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			expect(result.current.files[0].eol).toBe("LF");

			vi.mocked(readTextFile).mockResolvedValueOnce("line1\r\nline2");
			await act(async () => {
				await result.current.reloadFileIfClean("/test/file.ts");
			});

			expect(result.current.files[0].eol).toBe("CRLF");
		});
	});

	describe("updateContent", () => {
		it("should update file content and set isDirty when content changes", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			expect(result.current.files[0].isDirty).toBe(false);

			act(() => {
				result.current.updateContent("/test/file.ts", "modified content");
			});

			expect(result.current.files[0].content).toBe("modified content");
			expect(result.current.files[0].isDirty).toBe(true);
		});

		it("should set isDirty to false when content matches original", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateContent("/test/file.ts", "modified");
			});
			expect(result.current.files[0].isDirty).toBe(true);

			act(() => {
				result.current.updateContent("/test/file.ts", "file content");
			});
			expect(result.current.files[0].isDirty).toBe(false);
		});
	});

	describe("saveFile", () => {
		it("should save file and reset isDirty", async () => {
			const { writeTextFile } = await import("@tauri-apps/plugin-fs");
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateContent("/test/file.ts", "new content");
			});
			expect(result.current.files[0].isDirty).toBe(true);

			await act(async () => {
				await result.current.saveFile("/test/file.ts");
			});

			expect(writeTextFile).toHaveBeenCalledWith(
				"/test/file.ts",
				"new content",
			);
			expect(result.current.files[0].isDirty).toBe(false);
			expect(result.current.files[0].originalContent).toBe("new content");
		});
	});

	describe("closeFile", () => {
		it("should remove file from files list", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			expect(result.current.files).toHaveLength(1);

			act(() => {
				result.current.closeFile("/test/file.ts");
			});
			expect(result.current.files).toHaveLength(0);
		});
	});

	describe("updateFilePath", () => {
		it("should update file path, name, and language", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateFilePath("/test/file.ts", "/test/renamed.js");
			});

			expect(result.current.files[0].path).toBe("/test/renamed.js");
			expect(result.current.files[0].name).toBe("renamed.js");
			expect(result.current.files[0].language).toBe("javascript");
		});
	});

	describe("closeFilesByPrefix", () => {
		it("should close files matching the path prefix", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/dir/a.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/dir/b.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/other.ts");
			});

			expect(result.current.files).toHaveLength(3);

			act(() => {
				result.current.closeFilesByPrefix("/test/dir");
			});

			expect(result.current.files).toHaveLength(1);
			expect(result.current.files[0].path).toBe("/test/other.ts");
		});
	});

	describe("markExternalChange / clearExternalChange", () => {
		it("should set hasExternalChange flag", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			expect(result.current.files[0].hasExternalChange).toBeFalsy();

			act(() => {
				result.current.markExternalChange("/test/file.ts");
			});
			expect(result.current.files[0].hasExternalChange).toBe(true);
		});

		it("should clear hasExternalChange flag", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.markExternalChange("/test/file.ts");
			});
			expect(result.current.files[0].hasExternalChange).toBe(true);

			act(() => {
				result.current.clearExternalChange("/test/file.ts");
			});
			expect(result.current.files[0].hasExternalChange).toBe(false);
		});

		it("should clear hasExternalChange on save", async () => {
			const { result } = renderHook(() => useFileContents());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateContent("/test/file.ts", "edited");
				result.current.markExternalChange("/test/file.ts");
			});

			await act(async () => {
				await result.current.saveFile("/test/file.ts");
			});

			expect(result.current.files[0].hasExternalChange).toBe(false);
			expect(result.current.files[0].isDirty).toBe(false);
		});
	});

	describe("createUntitledFile", () => {
		it("should create an untitled file and return its path", () => {
			const { result } = renderHook(() => useFileContents());

			let path = "";
			act(() => {
				path = result.current.createUntitledFile();
			});

			expect(path).toBe("untitled:Untitled-1");
			expect(result.current.files).toHaveLength(1);
			expect(result.current.files[0].isDirty).toBe(true);
			expect(result.current.files[0].isUntitled).toBe(true);
		});
	});
});
