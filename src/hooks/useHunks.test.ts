import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useHunks } from "./useHunks";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

const { invoke } = await import("@tauri-apps/api/core");
const mockInvoke = vi.mocked(invoke);

function makeDiffResult(
	hunks: Array<{
		index: number;
		oldStart: number;
		oldLines: number;
		newStart: number;
		newLines: number;
		lines: string[];
	}>,
	changeGroups: Array<{
		groupIndex: number;
		hunkIndex: number;
		newStart: number;
		newEnd: number;
		lineOffsetStart: number;
		lineOffsetEnd: number;
	}>,
) {
	return { hunks, changeGroups };
}

const EMPTY_RESULT = makeDiffResult([], []);

const SINGLE_HUNK_RESULT = makeDiffResult(
	[
		{
			index: 0,
			oldStart: 1,
			oldLines: 1,
			newStart: 1,
			newLines: 1,
			lines: ["-line1", "+modified"],
		},
	],
	[
		{
			groupIndex: 0,
			hunkIndex: 0,
			newStart: 1,
			newEnd: 1,
			lineOffsetStart: 0,
			lineOffsetEnd: 1,
		},
	],
);

const TWO_GROUPS_RESULT = makeDiffResult(
	[
		{
			index: 0,
			oldStart: 1,
			oldLines: 3,
			newStart: 1,
			newLines: 3,
			lines: [" a", "-b", "+B", " c"],
		},
		{
			index: 1,
			oldStart: 17,
			oldLines: 3,
			newStart: 17,
			newLines: 3,
			lines: [" q", "-r", "+R", " s"],
		},
	],
	[
		{
			groupIndex: 0,
			hunkIndex: 0,
			newStart: 2,
			newEnd: 2,
			lineOffsetStart: 1,
			lineOffsetEnd: 2,
		},
		{
			groupIndex: 1,
			hunkIndex: 1,
			newStart: 18,
			newEnd: 18,
			lineOffsetStart: 1,
			lineOffsetEnd: 2,
		},
	],
);

describe("useHunks", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	it("should return empty hunks for identical content", async () => {
		mockInvoke.mockResolvedValue(EMPTY_RESULT);
		const { result } = renderHook(() => useHunks("hello\n", "hello\n"));

		await waitFor(() => {
			expect(result.current.hunks).toEqual([]);
		});
		expect(result.current.total).toBe(0);
		expect(result.current.currentGroup).toBeNull();
	});

	it("should compute hunks for different content", async () => {
		mockInvoke.mockResolvedValue(SINGLE_HUNK_RESULT);
		const { result } = renderHook(() =>
			useHunks("line1\nline2\n", "line1\nmodified\n"),
		);

		await waitFor(() => {
			expect(result.current.hunks.length).toBeGreaterThan(0);
		});
		expect(result.current.total).toBeGreaterThan(0);
	});

	it("should navigate to next hunk", async () => {
		mockInvoke.mockResolvedValue(TWO_GROUPS_RESULT);
		const { result } = renderHook(() => useHunks("a\n", "b\n"));

		await waitFor(() => {
			expect(result.current.total).toBe(2);
		});
		expect(result.current.currentIndex).toBe(0);

		act(() => {
			result.current.goToNext();
		});
		expect(result.current.currentIndex).toBe(1);
	});

	it("should wrap around when navigating past last hunk", async () => {
		mockInvoke.mockResolvedValue(TWO_GROUPS_RESULT);
		const { result } = renderHook(() => useHunks("a\n", "b\n"));

		await waitFor(() => {
			expect(result.current.total).toBe(2);
		});

		act(() => {
			result.current.goToNext();
		});
		act(() => {
			result.current.goToNext();
		});
		expect(result.current.currentIndex).toBe(0);
	});

	it("should navigate to previous hunk with wrap", async () => {
		mockInvoke.mockResolvedValue(TWO_GROUPS_RESULT);
		const { result } = renderHook(() => useHunks("a\n", "b\n"));

		await waitFor(() => {
			expect(result.current.total).toBe(2);
		});

		act(() => {
			result.current.goToPrev();
		});
		expect(result.current.currentIndex).toBe(result.current.total - 1);
	});

	it("should go to specific hunk index", async () => {
		mockInvoke.mockResolvedValue(TWO_GROUPS_RESULT);
		const { result } = renderHook(() => useHunks("a\n", "b\n"));

		await waitFor(() => {
			expect(result.current.total).toBe(2);
		});

		act(() => {
			result.current.goTo(1);
		});
		expect(result.current.currentIndex).toBe(1);
	});

	it("should return currentGroup when hunks exist", async () => {
		mockInvoke.mockResolvedValue(SINGLE_HUNK_RESULT);
		const { result } = renderHook(() => useHunks("line1\n", "modified\n"));

		await waitFor(() => {
			expect(result.current.currentGroup).not.toBeNull();
		});
		expect(result.current.currentGroup?.groupIndex).toBe(0);
	});
});
