import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AheadBehind } from "@/types/git";
import { useAheadBehind } from "./useAheadBehind";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useAheadBehind", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should return null when rootPath is null", () => {
		const { result } = renderHook(() => useAheadBehind(null));
		expect(result.current).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should fetch ahead/behind data", async () => {
		const mockData: AheadBehind = {
			ahead: 3,
			behind: 1,
			has_upstream: true,
		};
		mockInvoke.mockResolvedValue(mockData);

		const { result } = renderHook(() => useAheadBehind("/test/repo"));

		await waitFor(() => {
			expect(result.current).toEqual(mockData);
		});

		expect(mockInvoke).toHaveBeenCalledWith("get_current_branch_ahead_behind", {
			repoPath: "/test/repo",
		});
	});

	it("should return null on invoke error", async () => {
		mockInvoke.mockRejectedValue(new Error("fail"));

		const { result } = renderHook(() => useAheadBehind("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalled();
		});

		expect(result.current).toBeNull();
	});

	it("should refetch when externalRefreshKey changes", async () => {
		const mockData: AheadBehind = {
			ahead: 1,
			behind: 0,
			has_upstream: true,
		};
		mockInvoke.mockResolvedValue(mockData);

		const { result, rerender } = renderHook(
			({ key }: { key: number }) => useAheadBehind("/test/repo", key),
			{ initialProps: { key: 0 } },
		);

		await waitFor(() => {
			expect(result.current).toEqual(mockData);
		});

		const updatedData: AheadBehind = {
			ahead: 2,
			behind: 0,
			has_upstream: true,
		};
		mockInvoke.mockResolvedValue(updatedData);

		rerender({ key: 1 });

		await waitFor(() => {
			expect(result.current).toEqual(updatedData);
		});

		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});
});
