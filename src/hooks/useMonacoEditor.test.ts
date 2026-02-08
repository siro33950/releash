import { loader } from "@monaco-editor/react";
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useMonacoEditor } from "./useMonacoEditor";

describe("useMonacoEditor", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should initialize with container ref", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };

		renderHook(() => useMonacoEditor(containerRef));

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});

	it("should not initialize without container", () => {
		const containerRef = { current: null };

		renderHook(() => useMonacoEditor(containerRef));

		expect(loader.init).not.toHaveBeenCalled();
	});

	it("should accept options", async () => {
		const container = document.createElement("div");
		const containerRef = { current: container };
		const onChange = vi.fn();

		renderHook(() =>
			useMonacoEditor(containerRef, {
				defaultValue: "test code",
				language: "javascript",
				onChange,
			}),
		);

		await vi.waitFor(() => {
			expect(loader.init).toHaveBeenCalled();
		});
	});
});
