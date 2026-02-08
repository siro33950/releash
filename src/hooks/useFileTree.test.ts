import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useFileTree } from "./useFileTree";

const mockReadDir = vi.fn();

vi.mock("@tauri-apps/plugin-fs", () => ({
	readDir: (...args: unknown[]) => mockReadDir(...args),
}));

describe("useFileTree", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should return empty tree when rootPath is null", () => {
		const { result } = renderHook(() => useFileTree({ rootPath: null }));

		expect(result.current.tree).toEqual([]);
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBeNull();
	});

	it("should load root directory contents", async () => {
		mockReadDir.mockResolvedValue([
			{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
			{
				name: "package.json",
				isDirectory: false,
				isFile: true,
				isSymlink: false,
			},
		]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.tree).toHaveLength(2);
		expect(result.current.tree[0].name).toBe("src");
		expect(result.current.tree[0].type).toBe("folder");
		expect(result.current.tree[1].name).toBe("package.json");
		expect(result.current.tree[1].type).toBe("file");
	});

	it("should sort folders before files", async () => {
		mockReadDir.mockResolvedValue([
			{ name: "index.ts", isDirectory: false, isFile: true, isSymlink: false },
			{
				name: "components",
				isDirectory: true,
				isFile: false,
				isSymlink: false,
			},
			{ name: "utils", isDirectory: true, isFile: false, isSymlink: false },
			{ name: "app.tsx", isDirectory: false, isFile: true, isSymlink: false },
		]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.tree[0].type).toBe("folder");
		expect(result.current.tree[1].type).toBe("folder");
		expect(result.current.tree[2].type).toBe("file");
		expect(result.current.tree[3].type).toBe("file");
	});

	it("should filter hidden files when showHidden is false", async () => {
		mockReadDir.mockResolvedValue([
			{ name: ".git", isDirectory: true, isFile: false, isSymlink: false },
			{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
			{ name: ".env", isDirectory: false, isFile: true, isSymlink: false },
		]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project", showHidden: false }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.tree).toHaveLength(1);
		expect(result.current.tree[0].name).toBe("src");
	});

	it("should toggle folder expansion", async () => {
		mockReadDir
			.mockResolvedValueOnce([
				{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
			])
			.mockResolvedValueOnce([
				{
					name: "index.ts",
					isDirectory: false,
					isFile: true,
					isSymlink: false,
				},
			]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.expandedPaths.has("/test/project/src")).toBe(false);

		await act(async () => {
			await result.current.toggleExpand("/test/project/src");
		});

		expect(result.current.expandedPaths.has("/test/project/src")).toBe(true);
		expect(result.current.tree[0].children).toHaveLength(1);

		await act(async () => {
			await result.current.toggleExpand("/test/project/src");
		});

		expect(result.current.expandedPaths.has("/test/project/src")).toBe(false);
	});

	it("should handle errors gracefully", async () => {
		mockReadDir.mockRejectedValue(new Error("Permission denied"));

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.error).toBe("Permission denied");
		expect(result.current.tree).toEqual([]);
	});

	it("should refresh tree when refresh is called", async () => {
		mockReadDir.mockResolvedValue([
			{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
		]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(mockReadDir).toHaveBeenCalledTimes(1);

		await act(async () => {
			await result.current.refresh();
		});

		expect(mockReadDir).toHaveBeenCalledTimes(2);
	});

	it("should collapse all expanded folders when collapseAll is called", async () => {
		mockReadDir
			.mockResolvedValueOnce([
				{ name: "src", isDirectory: true, isFile: false, isSymlink: false },
				{ name: "lib", isDirectory: true, isFile: false, isSymlink: false },
			])
			.mockResolvedValueOnce([
				{
					name: "index.ts",
					isDirectory: false,
					isFile: true,
					isSymlink: false,
				},
			])
			.mockResolvedValueOnce([
				{
					name: "utils.ts",
					isDirectory: false,
					isFile: true,
					isSymlink: false,
				},
			]);

		const { result } = renderHook(() =>
			useFileTree({ rootPath: "/test/project" }),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		await act(async () => {
			await result.current.toggleExpand("/test/project/src");
		});
		await act(async () => {
			await result.current.toggleExpand("/test/project/lib");
		});

		expect(result.current.expandedPaths.has("/test/project/src")).toBe(true);
		expect(result.current.expandedPaths.has("/test/project/lib")).toBe(true);

		act(() => {
			result.current.collapseAll();
		});

		expect(result.current.expandedPaths.size).toBe(0);
		expect(result.current.expandedPaths.has("/test/project/src")).toBe(false);
		expect(result.current.expandedPaths.has("/test/project/lib")).toBe(false);
	});
});
