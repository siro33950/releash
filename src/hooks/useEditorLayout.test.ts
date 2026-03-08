import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useEditorLayout } from "./useEditorLayout";

describe("useEditorLayout", () => {
	it("starts with empty state when no initialState provided", () => {
		const { result } = renderHook(() => useEditorLayout());
		expect(result.current.tabs).toEqual([]);
		expect(result.current.activeTabId).toBe("");
	});

	it("uses initialState when provided", () => {
		const initialState = {
			tabs: [
				{
					id: "editor:src/main.rs",
					path: "src/main.rs",
					name: "main.rs",
					isDirty: false,
					closable: true,
					draggable: true,
				},
				{
					id: "editor:src/lib.rs",
					path: "src/lib.rs",
					name: "lib.rs",
					isDirty: false,
					closable: true,
					draggable: true,
				},
			],
			activeTabId: "editor:src/main.rs",
		};

		const { result } = renderHook(() =>
			useEditorLayout(undefined, initialState),
		);
		expect(result.current.tabs).toHaveLength(2);
		expect(result.current.tabs[0].name).toBe("main.rs");
		expect(result.current.activeTabId).toBe("editor:src/main.rs");
	});

	it("can add tabs on top of initialState", () => {
		const initialState = {
			tabs: [
				{
					id: "editor:src/main.rs",
					path: "src/main.rs",
					name: "main.rs",
					isDirty: false,
					closable: true,
					draggable: true,
				},
			],
			activeTabId: "editor:src/main.rs",
		};

		const { result } = renderHook(() =>
			useEditorLayout(undefined, initialState),
		);

		act(() => {
			result.current.addTab("src/new.rs", "new.rs", false);
		});

		expect(result.current.tabs).toHaveLength(2);
		expect(result.current.activeTabId).toBe("editor:src/new.rs");
	});

	it("closeTab calls onTabClose and removes if not blocked", () => {
		const onTabClose = vi.fn(() => false);
		const initialState = {
			tabs: [
				{
					id: "editor:a.rs",
					path: "a.rs",
					name: "a.rs",
					isDirty: false,
					closable: true,
					draggable: true,
				},
			],
			activeTabId: "editor:a.rs",
		};

		const { result } = renderHook(() =>
			useEditorLayout(onTabClose, initialState),
		);

		act(() => {
			result.current.closeTab("a.rs");
		});

		expect(onTabClose).toHaveBeenCalledWith("a.rs");
		expect(result.current.tabs).toHaveLength(0);
	});
});
