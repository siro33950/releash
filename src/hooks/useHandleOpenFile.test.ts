import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { TabInfo } from "@/types/editor";
import { useHandleOpenFile } from "./useHandleOpenFile";

function createTabInfo(overrides: Partial<TabInfo> = {}): TabInfo {
	return {
		path: "src/main.ts",
		name: "main.ts",
		content: "",
		originalContent: "",
		isDirty: false,
		language: "typescript",
		eol: "LF",
		...overrides,
	};
}

describe("useHandleOpenFile", () => {
	it("should call openFile, addTab, and onSwitchToEditor for a new file", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi.fn().mockReturnValue(undefined);
		const addTab = vi.fn();
		const onSwitchToEditor = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab, onSwitchToEditor }),
		);

		await act(() => result.current("src/main.ts"));

		expect(openFile).toHaveBeenCalledWith("src/main.ts");
		expect(addTab).toHaveBeenCalledWith("src/main.ts", "main.ts", false);
		expect(onSwitchToEditor).toHaveBeenCalledOnce();
	});

	it("should call onSwitchToEditor for an already-opened file", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi
			.fn()
			.mockReturnValue(createTabInfo({ isDirty: false }));
		const addTab = vi.fn();
		const onSwitchToEditor = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab, onSwitchToEditor }),
		);

		await act(() => result.current("src/main.ts"));

		expect(onSwitchToEditor).toHaveBeenCalledOnce();
		expect(addTab).toHaveBeenCalledWith("src/main.ts", "main.ts", false);
	});

	it("should pass isDirty: true to addTab for a dirty file", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi
			.fn()
			.mockReturnValue(createTabInfo({ isDirty: true }));
		const addTab = vi.fn();
		const onSwitchToEditor = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab, onSwitchToEditor }),
		);

		await act(() => result.current("src/main.ts"));

		expect(addTab).toHaveBeenCalledWith("src/main.ts", "main.ts", true);
		expect(onSwitchToEditor).toHaveBeenCalledOnce();
	});

	it("should fire onSwitchToEditor on every call even for the same file", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi
			.fn()
			.mockReturnValue(createTabInfo({ isDirty: false }));
		const addTab = vi.fn();
		const onSwitchToEditor = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab, onSwitchToEditor }),
		);

		await act(() => result.current("src/main.ts"));
		await act(() => result.current("src/main.ts"));

		expect(onSwitchToEditor).toHaveBeenCalledTimes(2);
	});

	it("should call addTab and onSwitchToEditor for a non-active tab", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi
			.fn()
			.mockReturnValue(
				createTabInfo({ path: "src/other.ts", name: "other.ts" }),
			);
		const addTab = vi.fn();
		const onSwitchToEditor = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab, onSwitchToEditor }),
		);

		await act(() => result.current("src/other.ts"));

		expect(addTab).toHaveBeenCalledWith("src/other.ts", "other.ts", false);
		expect(onSwitchToEditor).toHaveBeenCalledOnce();
	});

	it("should not throw when onSwitchToEditor is undefined", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi.fn().mockReturnValue(undefined);
		const addTab = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab }),
		);

		await expect(
			act(() => result.current("src/main.ts")),
		).resolves.not.toThrow();
		expect(addTab).toHaveBeenCalledWith("src/main.ts", "main.ts", false);
	});

	it("should extract filename correctly from Unix paths", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi.fn().mockReturnValue(undefined);
		const addTab = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab }),
		);

		await act(() => result.current("src/components/Button.tsx"));

		expect(addTab).toHaveBeenCalledWith(
			"src/components/Button.tsx",
			"Button.tsx",
			false,
		);
	});

	it("should normalize Windows paths to forward slashes", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi.fn().mockReturnValue(undefined);
		const addTab = vi.fn();

		const { result } = renderHook(() =>
			useHandleOpenFile({ openFile, getFileContent, addTab }),
		);

		await act(() => result.current("src\\components\\Button.tsx"));

		expect(openFile).toHaveBeenCalledWith("src/components/Button.tsx");
		expect(addTab).toHaveBeenCalledWith(
			"src/components/Button.tsx",
			"Button.tsx",
			false,
		);
	});

	it("should use the latest onSwitchToEditor after rerender", async () => {
		const openFile = vi.fn().mockResolvedValue(undefined);
		const getFileContent = vi.fn().mockReturnValue(undefined);
		const addTab = vi.fn();
		const firstOnSwitchToEditor = vi.fn();
		const secondOnSwitchToEditor = vi.fn();

		const { result, rerender } = renderHook(
			({ onSwitchToEditor }: { onSwitchToEditor?: () => void }) =>
				useHandleOpenFile({
					openFile,
					getFileContent,
					addTab,
					onSwitchToEditor,
				}),
			{
				initialProps: { onSwitchToEditor: firstOnSwitchToEditor },
			},
		);

		rerender({ onSwitchToEditor: secondOnSwitchToEditor });
		await act(() => result.current("src/main.ts"));

		expect(firstOnSwitchToEditor).not.toHaveBeenCalled();
		expect(secondOnSwitchToEditor).toHaveBeenCalledOnce();
	});
});
