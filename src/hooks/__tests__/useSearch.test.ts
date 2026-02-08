import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSearch } from "../useSearch";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const mockInvoke = vi.mocked(invoke);

describe("useSearch", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should return initial state", () => {
		const { result } = renderHook(() => useSearch("/root"));
		expect(result.current.result).toBeNull();
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBeNull();
	});

	it("should call invoke with correct params", async () => {
		const mockResult = {
			matches: [
				{
					path: "src/main.ts",
					line_number: 1,
					line_content: 'console.log("hello")',
					match_start: 0,
					match_end: 7,
				},
			],
			total_matches: 1,
			truncated: false,
		};
		mockInvoke.mockResolvedValueOnce(mockResult);

		const { result } = renderHook(() => useSearch("/root"));

		await act(async () => {
			await result.current.search("hello", {
				caseSensitive: true,
				isRegex: false,
			});
		});

		expect(mockInvoke).toHaveBeenCalledWith("search_files", {
			rootPath: "/root",
			pattern: "hello",
			caseSensitive: true,
			isRegex: false,
			maxResults: 1000,
		});
		expect(result.current.result).toEqual(mockResult);
		expect(result.current.loading).toBe(false);
	});

	it("should handle errors", async () => {
		mockInvoke.mockRejectedValueOnce(new Error("search failed"));

		const { result } = renderHook(() => useSearch("/root"));

		await act(async () => {
			await result.current.search("test");
		});

		expect(result.current.error).toBe("Error: search failed");
		expect(result.current.result).toBeNull();
	});

	it("should clear results", async () => {
		const mockResult = {
			matches: [],
			total_matches: 0,
			truncated: false,
		};
		mockInvoke.mockResolvedValueOnce(mockResult);

		const { result } = renderHook(() => useSearch("/root"));

		await act(async () => {
			await result.current.search("test");
		});
		expect(result.current.result).toEqual(mockResult);

		act(() => {
			result.current.clear();
		});
		expect(result.current.result).toBeNull();
	});

	it("should not search when rootPath is null", async () => {
		const { result } = renderHook(() => useSearch(null));

		await act(async () => {
			await result.current.search("test");
		});

		expect(mockInvoke).not.toHaveBeenCalled();
		expect(result.current.result).toBeNull();
	});

	it("should not search with empty pattern", async () => {
		const { result } = renderHook(() => useSearch("/root"));

		await act(async () => {
			await result.current.search("  ");
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});
});
