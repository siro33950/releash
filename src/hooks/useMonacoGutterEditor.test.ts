import { loader } from "@monaco-editor/react";
import { act, renderHook } from "@testing-library/react";
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

	it("should not call onContentChange during programmatic setValue", async () => {
		const OriginalIntersectionObserver = globalThis.IntersectionObserver;
		globalThis.IntersectionObserver = class IntersectionObserver {
			observe() {}
			unobserve() {}
			disconnect() {}
		} as unknown as typeof globalThis.IntersectionObserver;

		const monaco = await loader.init();

		// Add OverviewRulerLane needed by updateDecorations
		(monaco.editor as Record<string, unknown>).OverviewRulerLane = {
			Full: 7,
		};

		let contentChangeHandler: (() => void) | null = null;
		const editorInstance = {
			...monaco.editor.create(),
			onDidChangeModelContent: vi
				.fn()
				.mockImplementation((handler: () => void) => {
					contentChangeHandler = handler;
					return { dispose: vi.fn() };
				}),
			getValue: vi.fn().mockReturnValue("initial"),
			setValue: vi.fn(),
			getScrollTop: vi.fn().mockReturnValue(0),
			setScrollTop: vi.fn(),
			getPosition: vi.fn().mockReturnValue(null),
			setPosition: vi.fn(),
			deltaDecorations: vi.fn().mockReturnValue([]),
			onMouseDown: vi.fn().mockReturnValue({ dispose: vi.fn() }),
			onMouseMove: vi.fn().mockReturnValue({ dispose: vi.fn() }),
			onMouseUp: vi.fn().mockReturnValue({ dispose: vi.fn() }),
			onDidLayoutChange: vi.fn().mockReturnValue({ dispose: vi.fn() }),
			getDomNode: vi.fn().mockReturnValue(null),
			changeViewZones: vi.fn(),
			addAction: vi.fn(),
			dispose: vi.fn(),
			layout: vi.fn(),
		};
		vi.mocked(monaco.editor.create).mockReturnValue(editorInstance);

		const onContentChange = vi.fn();
		const container = document.createElement("div");
		const containerRef = { current: container };

		const { rerender } = renderHook(
			(props: { modifiedValue: string }) =>
				useMonacoGutterEditor(containerRef, {
					originalValue: "original",
					modifiedValue: props.modifiedValue,
					onContentChange,
				}),
			{ initialProps: { modifiedValue: "initial" } },
		);

		await vi.waitFor(() => {
			expect(contentChangeHandler).not.toBeNull();
		});

		onContentChange.mockClear();

		editorInstance.setValue.mockImplementation(() => {
			contentChangeHandler?.();
		});

		act(() => {
			rerender({ modifiedValue: "updated externally" });
		});

		expect(editorInstance.setValue).toHaveBeenCalledWith("updated externally");
		expect(onContentChange).not.toHaveBeenCalled();

		globalThis.IntersectionObserver = OriginalIntersectionObserver;
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
