import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import type { DiffTreeNode } from "@/types/review";
import { useFileNavigation } from "./useFileNavigation";

const mockedInvoke = vi.mocked(invoke);

function makeFileNode(path: string): DiffTreeNode {
	return {
		id: `file:${path}`,
		name: path.split("/").pop() ?? path,
		path,
		node_type: "file",
		status: "modified",
		additions: 0,
		deletions: 0,
		children: [],
	};
}

const sampleTree: DiffTreeNode[] = [
	makeFileNode("a.ts"),
	makeFileNode("b.ts"),
	makeFileNode("c.ts"),
];

describe("useFileNavigation", () => {
	it("should return empty navigation when currentFile is null", () => {
		const { result } = renderHook(() => useFileNavigation(sampleTree, null));
		expect(result.current.fileNavigation.total).toBe(0);
		expect(result.current.fileNavigation.prev_file).toBeNull();
		expect(result.current.fileNavigation.next_file).toBeNull();
	});

	it("should return empty navigation when tree is empty", () => {
		const { result } = renderHook(() => useFileNavigation([], "some-file.ts"));
		expect(result.current.fileNavigation.total).toBe(0);
	});

	it("should invoke get_file_navigation with tree and currentFile", async () => {
		mockedInvoke.mockResolvedValueOnce({
			current_index: 1,
			total: 3,
			prev_file: "a.ts",
			next_file: "c.ts",
		});

		const { result } = renderHook(() => useFileNavigation(sampleTree, "b.ts"));

		await waitFor(() => {
			expect(result.current.fileNavigation.total).toBe(3);
		});

		expect(mockedInvoke).toHaveBeenCalledWith("get_file_navigation", {
			tree: sampleTree,
			currentFile: "b.ts",
		});
		expect(result.current.fileNavigation.current_index).toBe(1);
		expect(result.current.fileNavigation.prev_file).toBe("a.ts");
		expect(result.current.fileNavigation.next_file).toBe("c.ts");
	});

	it("goToPrevFile returns prev_file path", async () => {
		mockedInvoke.mockResolvedValueOnce({
			current_index: 1,
			total: 3,
			prev_file: "a.ts",
			next_file: "c.ts",
		});

		const { result } = renderHook(() => useFileNavigation(sampleTree, "b.ts"));

		await waitFor(() => {
			expect(result.current.fileNavigation.total).toBe(3);
		});

		expect(result.current.goToPrevFile()).toBe("a.ts");
	});

	it("goToNextFile returns next_file path", async () => {
		mockedInvoke.mockResolvedValueOnce({
			current_index: 1,
			total: 3,
			prev_file: "a.ts",
			next_file: "c.ts",
		});

		const { result } = renderHook(() => useFileNavigation(sampleTree, "b.ts"));

		await waitFor(() => {
			expect(result.current.fileNavigation.total).toBe(3);
		});

		expect(result.current.goToNextFile()).toBe("c.ts");
	});

	it("should reset to empty when invoke fails", async () => {
		mockedInvoke.mockRejectedValueOnce(new Error("fail"));

		const { result } = renderHook(() => useFileNavigation(sampleTree, "b.ts"));

		await waitFor(() => {
			expect(mockedInvoke).toHaveBeenCalled();
		});

		expect(result.current.fileNavigation.total).toBe(0);
	});
});
