import { loader } from "@monaco-editor/react";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	computeDiff,
	createDiffDecorations,
	useMonacoGutterEditor,
} from "./useMonacoGutterEditor";

describe("computeDiff", () => {
	it("should return empty arrays when texts are identical", () => {
		const result = computeDiff("line1\nline2\n", "line1\nline2\n");
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([]);
	});

	it("should detect added lines", () => {
		const result = computeDiff("line1\nline2\n", "line1\nline2\nline3\n");
		expect(result.added).toEqual([3]);
		expect(result.deleted).toEqual([]);
	});

	it("should detect replacement as added lines", () => {
		const result = computeDiff(
			"line1\nline2\nline3\n",
			"line1\nmodified\nline3\n",
		);
		expect(result.added).toEqual([2]);
		expect(result.deleted).toEqual([]);
	});

	it("should not add out-of-range line numbers when deleting trailing lines", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nline2\n");
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([2]);
	});

	it("should mark deleted when deleting middle lines", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nline3\n");
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([2]);
	});

	it("should handle empty modified text", () => {
		const result = computeDiff("line1\nline2\n", "");
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([]);
	});

	it("should handle empty original text", () => {
		const result = computeDiff("", "line1\nline2\n");
		expect(result.added).toEqual([1, 2]);
		expect(result.deleted).toEqual([]);
	});

	it("should handle deleting all lines except first", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\n");
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([1]);
	});

	it("should detect deleted lines at end of file", () => {
		const result = computeDiff(
			"line1\nline2\nline3\nline4\n",
			"line1\nline2\n",
		);
		expect(result.added).toEqual([]);
		expect(result.deleted).toEqual([2]);
	});

	it("should detect replacement with deletion as added only", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nchanged\n");
		expect(result.added).toContain(2);
		expect(result.deleted).toEqual([]);
	});

	it("should handle multi-line replacement as added only", () => {
		const result = computeDiff("aaa\nbbb\nccc\n", "aaa\nxxx\nyyy\nccc\n");
		expect(result.added.length).toBeGreaterThan(0);
		expect(result.deleted).toEqual([]);
	});

	it("pure deletion produces deleted only", () => {
		const result = computeDiff("line1\nline2\nline3\n", "line1\nline3\n");
		expect(result.deleted).toEqual([2]);
		expect(result.added).toEqual([]);
	});

	it("should detect both added and deleted lines in mixed diff", () => {
		const result = computeDiff("a\nb\nc\nd\ne\n", "a\nX\nd\n");
		expect(result.added).toContain(2);
		expect(result.deleted.length).toBeGreaterThan(0);
	});
});

describe("createDiffDecorations", () => {
	let mockMonaco: Awaited<ReturnType<typeof loader.init>>;

	beforeEach(async () => {
		mockMonaco = await loader.init();
		(mockMonaco.editor as Record<string, unknown>).OverviewRulerLane = {
			Full: 7,
		};
	});

	it("should create gutter-added decorations for added lines", () => {
		const diff = { added: [1, 3], deleted: [] };
		const result = createDiffDecorations(diff, mockMonaco);

		expect(result).toHaveLength(2);
		for (const d of result) {
			expect(d.options.glyphMarginClassName).toBe("gutter-added");
			expect(d.options.overviewRuler?.color).toBe("#9ccc2c");
			expect(d.options.overviewRuler?.position).toBe(7);
		}
	});

	it("should create gutter-deleted decorations for deleted lines", () => {
		const diff = { added: [], deleted: [2, 4] };
		const result = createDiffDecorations(diff, mockMonaco);

		expect(result).toHaveLength(2);
		for (const d of result) {
			expect(d.options.glyphMarginClassName).toBe("gutter-deleted");
			expect(d.options.overviewRuler?.color).toBe("#ff0000");
			expect(d.options.overviewRuler?.position).toBe(7);
		}
	});

	it("should return empty array for empty diff", () => {
		const diff = { added: [], deleted: [] };
		const result = createDiffDecorations(diff, mockMonaco);

		expect(result).toEqual([]);
	});

	it("should create both added and deleted decorations for mixed diff", () => {
		const diff = { added: [2], deleted: [5] };
		const result = createDiffDecorations(diff, mockMonaco);

		expect(result).toHaveLength(2);
		expect(result[0].options.glyphMarginClassName).toBe("gutter-added");
		expect(result[1].options.glyphMarginClassName).toBe("gutter-deleted");
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
