import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { DiffLine } from "@/hooks/useDiffTokens";
import { useDiffSearch } from "./useDiffSearch";

const makeLine = (content: string): DiffLine => ({
	type: "context",
	oldLineNumber: null,
	newLineNumber: null,
	tokens: [{ content, color: undefined, offset: 0 }],
	content,
});

const sampleLines: DiffLine[] = [
	makeLine("hello world"),
	makeLine("foo bar baz"),
	makeLine("hello again"),
];

describe("useDiffSearch", () => {
	it("starts closed with empty query", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));
		expect(result.current.isOpen).toBe(false);
		expect(result.current.query).toBe("");
		expect(result.current.totalMatches).toBe(0);
	});

	it("opens and closes", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => result.current.open());
		expect(result.current.isOpen).toBe(true);

		act(() => result.current.close());
		expect(result.current.isOpen).toBe(false);
		expect(result.current.query).toBe("");
	});

	it("finds matches when query is set", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		expect(result.current.totalMatches).toBe(2);
		expect(result.current.currentIndex).toBe(0);
	});

	it("navigates to next match", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		act(() => result.current.goToNext());
		expect(result.current.currentIndex).toBe(1);
	});

	it("navigates to previous match", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		act(() => result.current.goToNext());
		expect(result.current.currentIndex).toBe(1);

		act(() => result.current.goToPrev());
		expect(result.current.currentIndex).toBe(0);
	});

	it("wraps around from last to first", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		act(() => result.current.goToNext());
		expect(result.current.currentIndex).toBe(1);

		act(() => result.current.goToNext());
		expect(result.current.currentIndex).toBe(0);
	});

	it("wraps around from first to last", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		act(() => result.current.goToPrev());
		expect(result.current.currentIndex).toBe(1);
	});

	it("resets currentIndex when query changes", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("hello");
		});

		act(() => result.current.goToNext());
		expect(result.current.currentIndex).toBe(1);

		act(() => result.current.setQuery("foo"));
		expect(result.current.currentIndex).toBe(0);
	});

	it("returns -1 currentIndex when no matches", () => {
		const { result } = renderHook(() => useDiffSearch(sampleLines));

		act(() => {
			result.current.open();
			result.current.setQuery("xyz");
		});

		expect(result.current.currentIndex).toBe(-1);
		expect(result.current.totalMatches).toBe(0);
	});
});
