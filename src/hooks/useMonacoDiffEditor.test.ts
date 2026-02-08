import { loader } from "@monaco-editor/react";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useMonacoDiffEditor } from "./useMonacoDiffEditor";

describe("useMonacoDiffEditor", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should initialize with container ref", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoDiffEditor(containerRef, {
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
			useMonacoDiffEditor(containerRef, {
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
			useMonacoDiffEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
				language: "javascript",
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});

	it("should accept renderSideBySide option for split mode", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoDiffEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
				renderSideBySide: true,
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});

	it("should accept renderSideBySide option for inline mode", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() =>
			useMonacoDiffEditor(containerRef, {
				originalValue: "original",
				modifiedValue: "modified",
				renderSideBySide: false,
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});
});
