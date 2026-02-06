import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useHunks } from "./useHunks";

describe("useHunks", () => {
	it("should return empty hunks for identical content", () => {
		const { result } = renderHook(() => useHunks("hello\n", "hello\n"));
		expect(result.current.hunks).toEqual([]);
		expect(result.current.total).toBe(0);
		expect(result.current.currentGroup).toBeNull();
	});

	it("should compute hunks for different content", () => {
		const { result } = renderHook(() =>
			useHunks("line1\nline2\n", "line1\nmodified\n"),
		);
		expect(result.current.hunks.length).toBeGreaterThan(0);
		expect(result.current.total).toBeGreaterThan(0);
	});

	it("should navigate to next hunk", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const { result } = renderHook(() => useHunks(lines, modified));

		expect(result.current.currentIndex).toBe(0);

		act(() => {
			result.current.goToNext();
		});
		expect(result.current.currentIndex).toBe(1);
	});

	it("should wrap around when navigating past last hunk", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const { result } = renderHook(() => useHunks(lines, modified));

		act(() => {
			result.current.goToNext();
		});
		act(() => {
			result.current.goToNext();
		});
		expect(result.current.currentIndex).toBe(0);
	});

	it("should navigate to previous hunk with wrap", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const { result } = renderHook(() => useHunks(lines, modified));

		act(() => {
			result.current.goToPrev();
		});
		expect(result.current.currentIndex).toBe(result.current.total - 1);
	});

	it("should go to specific hunk index", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const { result } = renderHook(() => useHunks(lines, modified));

		act(() => {
			result.current.goTo(1);
		});
		expect(result.current.currentIndex).toBe(1);
	});

	it("should return currentGroup when hunks exist", () => {
		const { result } = renderHook(() => useHunks("line1\n", "modified\n"));
		expect(result.current.currentGroup).not.toBeNull();
		expect(result.current.currentGroup?.groupIndex).toBe(0);
	});
});
