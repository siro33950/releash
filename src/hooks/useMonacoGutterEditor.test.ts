import { loader } from "@monaco-editor/react";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { computeDiff, useMonacoGutterEditor } from "./useMonacoGutterEditor";

describe("computeDiff", () => {
	it("should return empty arrays when texts are identical", () => {
		const result = computeDiff("line1\nline2\n", "line1\nline2\n");
		expect(result.added).toEqual([]);
		expect(result.modified).toEqual([]);
	});

	it("should detect added lines", () => {
		const result = computeDiff("line1\nline2\n", "line1\nline2\nline3\n");
		expect(result.added).toEqual([3]);
		expect(result.modified).toEqual([]);
	});

	it("should detect modified lines", () => {
		const result = computeDiff(
			"line1\nline2\nline3\n",
			"line1\nmodified\nline3\n",
		);
		expect(result.added).toEqual([2]);
		expect(result.modified).toEqual([2]);
	});

	it("should not add out-of-range line numbers when deleting trailing lines", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nline2\n");
		expect(result.added).toEqual([]);
		expect(result.modified).toEqual([]);
	});

	it("should mark modified when deleting middle lines", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nline3\n");
		expect(result.added).toEqual([]);
		expect(result.modified).toEqual([2]);
	});

	it("should handle empty modified text", () => {
		const result = computeDiff("line1\nline2\n", "");
		expect(result.added).toEqual([]);
		expect(result.modified).toEqual([]);
	});

	it("should handle empty original text", () => {
		const result = computeDiff("", "line1\nline2\n");
		expect(result.added).toEqual([1, 2]);
		expect(result.modified).toEqual([]);
	});

	it("should handle deleting all lines except first", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\n");
		expect(result.added).toEqual([]);
		expect(result.modified).toEqual([]);
	});
});

describe("useMonacoGutterEditor", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should initialize with container ref", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoGutterEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});

	it("should not initialize without container", () => {
		const containerRef = { current: null };

		renderHook(() =>
			useMonacoGutterEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
			}),
		);

		expect(loader.init).not.toHaveBeenCalled();
	});

	it("should accept language option", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoGutterEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
				language: "javascript",
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});

	it("should handle diff between original and modified content", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoGutterEditor(containerRef, {
				originalValue: "line1\nline2\nline3",
				modifiedValue: "line1\nmodified\nline3\nline4",
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});
});
