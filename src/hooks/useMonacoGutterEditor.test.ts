import { loader } from "@monaco-editor/react";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useMonacoGutterEditor } from "./useMonacoGutterEditor";

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
