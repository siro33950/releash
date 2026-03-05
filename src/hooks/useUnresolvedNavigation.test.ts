import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Thread } from "@/types/thread";
import { useUnresolvedNavigation } from "./useUnresolvedNavigation";

function makeThread(
	id: string,
	filePath: string,
	lineNumber: number,
	resolved = false,
): Thread {
	return {
		id,
		filePath,
		lineNumber,
		entries: [
			{
				id: `${id}-e0`,
				content: `content of ${id}`,
				isAi: false,
				createdAt: Date.now(),
			},
		],
		resolved,
		createdAt: Date.now(),
	};
}

describe("useUnresolvedNavigation", () => {
	it("returns total count of unresolved threads", () => {
		const threads = [
			makeThread("t1", "a.rs", 10),
			makeThread("t2", "a.rs", 20, true),
			makeThread("t3", "b.rs", 5),
		];
		const { result } = renderHook(() => useUnresolvedNavigation(threads));
		expect(result.current.total).toBe(2);
	});

	it("navigates to next unresolved thread", () => {
		const onNavigate = vi.fn();
		const threads = [makeThread("t1", "a.rs", 10), makeThread("t2", "b.rs", 5)];
		const { result } = renderHook(() =>
			useUnresolvedNavigation(threads, onNavigate),
		);

		act(() => result.current.goNext());
		expect(onNavigate).toHaveBeenCalledWith("a.rs", 10);
		expect(result.current.currentIndex).toBe(0);

		act(() => result.current.goNext());
		expect(onNavigate).toHaveBeenCalledWith("b.rs", 5);
		expect(result.current.currentIndex).toBe(1);
	});

	it("wraps around when reaching the end", () => {
		const onNavigate = vi.fn();
		const threads = [makeThread("t1", "a.rs", 10), makeThread("t2", "b.rs", 5)];
		const { result } = renderHook(() =>
			useUnresolvedNavigation(threads, onNavigate),
		);

		act(() => result.current.goNext());
		act(() => result.current.goNext());
		act(() => result.current.goNext());
		expect(result.current.currentIndex).toBe(0);
	});

	it("navigates to previous unresolved thread", () => {
		const onNavigate = vi.fn();
		const threads = [makeThread("t1", "a.rs", 10), makeThread("t2", "b.rs", 5)];
		const { result } = renderHook(() =>
			useUnresolvedNavigation(threads, onNavigate),
		);

		act(() => result.current.goPrev());
		// Starting at -1, goPrev should go to last (index 1)
		expect(result.current.currentIndex).toBe(1);
		expect(onNavigate).toHaveBeenCalledWith("b.rs", 5);
	});

	it("sorts by filePath then lineNumber", () => {
		const onNavigate = vi.fn();
		const threads = [
			makeThread("t1", "b.rs", 20),
			makeThread("t2", "a.rs", 30),
			makeThread("t3", "a.rs", 10),
		];
		const { result } = renderHook(() =>
			useUnresolvedNavigation(threads, onNavigate),
		);

		act(() => result.current.goNext());
		expect(onNavigate).toHaveBeenCalledWith("a.rs", 10);

		act(() => result.current.goNext());
		expect(onNavigate).toHaveBeenCalledWith("a.rs", 30);

		act(() => result.current.goNext());
		expect(onNavigate).toHaveBeenCalledWith("b.rs", 20);
	});

	it("handles empty threads", () => {
		const onNavigate = vi.fn();
		const { result } = renderHook(() =>
			useUnresolvedNavigation([], onNavigate),
		);
		expect(result.current.total).toBe(0);

		act(() => result.current.goNext());
		expect(onNavigate).not.toHaveBeenCalled();
	});

	it("excludes resolved threads", () => {
		const threads = [
			makeThread("t1", "a.rs", 10, true),
			makeThread("t2", "a.rs", 20, true),
		];
		const { result } = renderHook(() => useUnresolvedNavigation(threads));
		expect(result.current.total).toBe(0);
	});

	it("goToThread sets index correctly", () => {
		const threads = [makeThread("t1", "a.rs", 10), makeThread("t2", "b.rs", 5)];
		const { result } = renderHook(() => useUnresolvedNavigation(threads));

		act(() => result.current.goToThread("t2"));
		expect(result.current.currentIndex).toBe(1);
	});
});
