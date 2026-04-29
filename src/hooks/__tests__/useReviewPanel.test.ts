import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useReviewPanel } from "../useReviewPanel";

describe("useReviewPanel", () => {
	it("initializes with default values", () => {
		const { result } = renderHook(() => useReviewPanel());
		expect(result.current.diffBase).toBe("head");
		expect(result.current.diffMode).toBe("inline");
		expect(result.current.selectedFile).toBeNull();
		expect(result.current.selectedSection).toBe("changes");
	});

	it("initializes with provided options", () => {
		const { result } = renderHook(() =>
			useReviewPanel({
				initialDiffBase: "branch-base",
				initialDiffMode: "split",
			}),
		);
		expect(result.current.diffBase).toBe("branch-base");
		expect(result.current.diffMode).toBe("split");
	});

	it("switches diffBase from head to branch-base", () => {
		const { result } = renderHook(() => useReviewPanel());
		expect(result.current.diffBase).toBe("head");

		act(() => {
			result.current.setDiffBase("branch-base");
		});

		expect(result.current.diffBase).toBe("branch-base");
	});

	it("switches diffBase from branch-base to head", () => {
		const { result } = renderHook(() =>
			useReviewPanel({ initialDiffBase: "branch-base" }),
		);
		expect(result.current.diffBase).toBe("branch-base");

		act(() => {
			result.current.setDiffBase("head");
		});

		expect(result.current.diffBase).toBe("head");
	});

	it("changes diffMode", () => {
		const { result } = renderHook(() => useReviewPanel());

		act(() => {
			result.current.setDiffMode("gutter");
		});
		expect(result.current.diffMode).toBe("gutter");

		act(() => {
			result.current.setDiffMode("split");
		});
		expect(result.current.diffMode).toBe("split");
	});

	it("selects and deselects a file", () => {
		const { result } = renderHook(() => useReviewPanel());

		act(() => {
			result.current.selectFile("src/main.ts");
		});
		expect(result.current.selectedFile).toBe("src/main.ts");

		act(() => {
			result.current.selectFile(null);
		});
		expect(result.current.selectedFile).toBeNull();
	});

	it("updates selectedSection when selectFile is called with section", () => {
		const { result } = renderHook(() => useReviewPanel());
		expect(result.current.selectedSection).toBe("changes");

		act(() => {
			result.current.selectFile("src/main.ts", "staged");
		});
		expect(result.current.selectedFile).toBe("src/main.ts");
		expect(result.current.selectedSection).toBe("staged");

		act(() => {
			result.current.selectFile("src/app.ts", "changes");
		});
		expect(result.current.selectedFile).toBe("src/app.ts");
		expect(result.current.selectedSection).toBe("changes");
	});

	it("initializes selectedFile with initialSelectedFile option", () => {
		const { result } = renderHook(() =>
			useReviewPanel({
				initialSelectedFile: "src/main.rs",
			}),
		);
		expect(result.current.selectedFile).toBe("src/main.rs");
	});

	it("initializes selectedFile as null when initialSelectedFile is null", () => {
		const { result } = renderHook(() =>
			useReviewPanel({
				initialSelectedFile: null,
			}),
		);
		expect(result.current.selectedFile).toBeNull();
	});

	it("preserves selectedSection when selectFile is called without section", () => {
		const { result } = renderHook(() => useReviewPanel());

		act(() => {
			result.current.selectFile("src/main.ts", "staged");
		});
		expect(result.current.selectedSection).toBe("staged");

		act(() => {
			result.current.selectFile("src/other.ts");
		});
		expect(result.current.selectedFile).toBe("src/other.ts");
		expect(result.current.selectedSection).toBe("staged");
	});
});
