import { readTextFile } from "@tauri-apps/plugin-fs";
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { detectEol, useEditorTabs } from "./useEditorTabs";

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

describe("useEditorTabs", () => {
	describe("eol detection", () => {
		it("should set eol to LF when file has LF line endings", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\nline2");
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/lf.ts");
			});

			expect(result.current.tabs[0].eol).toBe("LF");
		});

		it("should set eol to CRLF when file has CRLF line endings", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\r\nline2");
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/crlf.ts");
			});

			expect(result.current.tabs[0].eol).toBe("CRLF");
		});

		it("should update eol on reloadTabIfClean", async () => {
			vi.mocked(readTextFile).mockResolvedValueOnce("line1\nline2");
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			expect(result.current.tabs[0].eol).toBe("LF");

			vi.mocked(readTextFile).mockResolvedValueOnce("line1\r\nline2");
			await act(async () => {
				await result.current.reloadTabIfClean("/test/file.ts");
			});

			expect(result.current.tabs[0].eol).toBe("CRLF");
		});
	});

	describe("updateTabContent", () => {
		it("should update tab content and set isDirty when content changes", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			expect(result.current.tabs).toHaveLength(1);
			expect(result.current.tabs[0].isDirty).toBe(false);

			act(() => {
				result.current.updateTabContent("/test/file.ts", "modified content");
			});

			expect(result.current.tabs[0].content).toBe("modified content");
			expect(result.current.tabs[0].isDirty).toBe(true);
		});

		it("should set isDirty to false when content matches original", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateTabContent("/test/file.ts", "modified");
			});
			expect(result.current.tabs[0].isDirty).toBe(true);

			act(() => {
				result.current.updateTabContent("/test/file.ts", "file content");
			});
			expect(result.current.tabs[0].isDirty).toBe(false);
		});
	});

	describe("saveFile", () => {
		it("should save file and reset isDirty", async () => {
			const { writeTextFile } = await import("@tauri-apps/plugin-fs");
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateTabContent("/test/file.ts", "new content");
			});
			expect(result.current.tabs[0].isDirty).toBe(true);

			await act(async () => {
				await result.current.saveFile("/test/file.ts");
			});

			expect(writeTextFile).toHaveBeenCalledWith(
				"/test/file.ts",
				"new content",
			);
			expect(result.current.tabs[0].isDirty).toBe(false);
			expect(result.current.tabs[0].originalContent).toBe("new content");
		});
	});

	describe("updateTabPath", () => {
		it("should update tab path, name, and language", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			act(() => {
				result.current.updateTabPath("/test/file.ts", "/test/renamed.js");
			});

			expect(result.current.tabs[0].path).toBe("/test/renamed.js");
			expect(result.current.tabs[0].name).toBe("renamed.js");
			expect(result.current.tabs[0].language).toBe("javascript");
		});

		it("should update active tab path when the active tab is renamed", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});

			expect(result.current.activeTab?.path).toBe("/test/file.ts");

			act(() => {
				result.current.updateTabPath("/test/file.ts", "/test/renamed.ts");
			});

			expect(result.current.activeTab?.path).toBe("/test/renamed.ts");
		});
	});

	describe("closeTabsByPrefix", () => {
		it("should close tabs matching the path prefix", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/dir/a.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/dir/b.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/other.ts");
			});

			expect(result.current.tabs).toHaveLength(3);

			act(() => {
				result.current.closeTabsByPrefix("/test/dir");
			});

			expect(result.current.tabs).toHaveLength(1);
			expect(result.current.tabs[0].path).toBe("/test/other.ts");
		});

		it("should close exact match tab", async () => {
			const { result } = renderHook(() => useEditorTabs());

			await act(async () => {
				await result.current.openFile("/test/file.ts");
			});
			await act(async () => {
				await result.current.openFile("/test/other.ts");
			});

			act(() => {
				result.current.closeTabsByPrefix("/test/file.ts");
			});

			expect(result.current.tabs).toHaveLength(1);
			expect(result.current.tabs[0].path).toBe("/test/other.ts");
		});
	});
});
