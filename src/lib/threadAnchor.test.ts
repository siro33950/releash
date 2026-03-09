import { describe, expect, it } from "vitest";
import type { Thread } from "@/types/thread";
import {
	createLineAnchor,
	recalculateThreadAnchors,
	resolveAnchor,
} from "./threadAnchor";

describe("createLineAnchor", () => {
	const fileContent = [
		"line 1",
		"line 2",
		"line 3",
		"line 4",
		"line 5",
		"line 6",
		"line 7",
		"line 8",
	].join("\n");

	it("should capture target line and 3 lines of context for a middle line", () => {
		const anchor = createLineAnchor(fileContent, 5);
		expect(anchor.targetLine).toBe("line 5");
		expect(anchor.contextBefore).toEqual(["line 2", "line 3", "line 4"]);
		expect(anchor.contextAfter).toEqual(["line 6", "line 7", "line 8"]);
		expect(anchor.originalLineNumber).toBe(5);
	});

	it("should have fewer contextBefore lines near the start of the file", () => {
		const anchor = createLineAnchor(fileContent, 1);
		expect(anchor.targetLine).toBe("line 1");
		expect(anchor.contextBefore).toEqual([]);
		expect(anchor.contextAfter).toEqual(["line 2", "line 3", "line 4"]);
		expect(anchor.originalLineNumber).toBe(1);
	});

	it("should have fewer contextBefore for line 2", () => {
		const anchor = createLineAnchor(fileContent, 2);
		expect(anchor.targetLine).toBe("line 2");
		expect(anchor.contextBefore).toEqual(["line 1"]);
	});

	it("should have fewer contextAfter near the end of the file", () => {
		const anchor = createLineAnchor(fileContent, 8);
		expect(anchor.targetLine).toBe("line 8");
		expect(anchor.contextBefore).toEqual(["line 5", "line 6", "line 7"]);
		expect(anchor.contextAfter).toEqual([]);
		expect(anchor.originalLineNumber).toBe(8);
	});

	it("should have fewer contextAfter for second-to-last line", () => {
		const anchor = createLineAnchor(fileContent, 7);
		expect(anchor.targetLine).toBe("line 7");
		expect(anchor.contextAfter).toEqual(["line 8"]);
	});
});

describe("resolveAnchor", () => {
	it("should find the target line when it has moved down", () => {
		const anchor = createLineAnchor(
			["a", "b", "target", "d", "e"].join("\n"),
			3,
		);
		const current = ["x", "y", "a", "b", "target", "d", "e"].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBe(5);
	});

	it("should find the target line when it has moved up", () => {
		const anchor = createLineAnchor(
			["a", "b", "c", "target", "e", "f"].join("\n"),
			4,
		);
		const current = ["a", "target", "e", "f"].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBe(2);
	});

	it("should return null when the target line is deleted", () => {
		const anchor = createLineAnchor(
			["a", "b", "target", "d", "e"].join("\n"),
			3,
		);
		const current = ["a", "b", "d", "e"].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBeNull();
	});

	it("should track via trim match when indentation changes", () => {
		const anchor = createLineAnchor(
			["a", "b", "  target line", "d", "e"].join("\n"),
			3,
		);
		const current = ["a", "b", "    target line", "d", "e"].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBe(3);
	});

	it("should distinguish duplicate lines using context", () => {
		const original = [
			"header1",
			"duplicate",
			"after-first",
			"header2",
			"duplicate",
			"after-second",
		].join("\n");
		const anchor = createLineAnchor(original, 5);
		expect(anchor.targetLine).toBe("duplicate");
		expect(anchor.contextBefore).toContain("header2");

		const current = [
			"new-line",
			"header1",
			"duplicate",
			"after-first",
			"new-line2",
			"header2",
			"duplicate",
			"after-second",
		].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBe(7);
	});

	it("should prefer the line closer to originalLineNumber on tie", () => {
		const anchor = {
			targetLine: "same",
			contextBefore: [],
			contextAfter: [],
			originalLineNumber: 3,
		};
		const current = ["same", "x", "same", "y", "same"].join("\n");
		const result = resolveAnchor(anchor, current);
		expect(result).toBe(3);
	});
});

describe("recalculateThreadAnchors", () => {
	function makeThread(
		id: string,
		filePath: string,
		lineNumber: number,
		anchor?: Thread["anchor"],
	): Thread {
		return {
			id,
			filePath,
			lineNumber,
			entries: [
				{
					id: "e1",
					content: "test",
					isAi: false,
					createdAt: Date.now(),
				},
			],
			resolved: false,
			createdAt: Date.now(),
			...(anchor != null && { anchor }),
		};
	}

	it("should update lineNumber for threads with anchors", () => {
		const anchor = createLineAnchor(["a", "b", "target", "d"].join("\n"), 3);
		const threads = [makeThread("t1", "file.ts", 3, anchor)];
		const current = ["x", "a", "b", "target", "d"].join("\n");
		const result = recalculateThreadAnchors(threads, "file.ts", current);
		expect(result[0].lineNumber).toBe(4);
	});

	it("should not modify threads without anchors", () => {
		const threads = [makeThread("t1", "file.ts", 3)];
		const current = ["x", "a", "b", "target", "d"].join("\n");
		const result = recalculateThreadAnchors(threads, "file.ts", current);
		expect(result[0]).toBe(threads[0]);
	});

	it("should not modify threads for different files", () => {
		const anchor = createLineAnchor(["a", "b", "target", "d"].join("\n"), 3);
		const threads = [makeThread("t1", "other.ts", 3, anchor)];
		const current = ["x", "a", "b", "target", "d"].join("\n");
		const result = recalculateThreadAnchors(threads, "file.ts", current);
		expect(result[0]).toBe(threads[0]);
	});

	it("should handle multiple threads correctly", () => {
		const original = ["a", "b", "target1", "c", "target2", "d"].join("\n");
		const anchor1 = createLineAnchor(original, 3);
		const anchor2 = createLineAnchor(original, 5);
		const threads = [
			makeThread("t1", "file.ts", 3, anchor1),
			makeThread("t2", "file.ts", 5, anchor2),
		];
		const current = ["x", "a", "b", "target1", "c", "target2", "d"].join("\n");
		const result = recalculateThreadAnchors(threads, "file.ts", current);
		expect(result[0].lineNumber).toBe(4);
		expect(result[1].lineNumber).toBe(6);
	});

	it("should keep lineNumber unchanged if anchor resolves to same line", () => {
		const original = ["a", "b", "target", "d"].join("\n");
		const anchor = createLineAnchor(original, 3);
		const threads = [makeThread("t1", "file.ts", 3, anchor)];
		const result = recalculateThreadAnchors(threads, "file.ts", original);
		expect(result[0]).toBe(threads[0]);
	});

	it("recalculates line numbers when lines are inserted before anchored threads", () => {
		const original = [
			"# Plan",
			"",
			"## Step 1",
			"Do something",
			"",
			"## Step 2",
		].join("\n");
		const anchor3 = createLineAnchor(original, 3);
		const anchor6 = createLineAnchor(original, 6);
		const threads = [
			makeThread("t1", "workflow://plan", 3, anchor3),
			makeThread("t2", "workflow://plan", 6, anchor6),
		];
		const updated = [
			"# Plan",
			"",
			"Added line A",
			"Added line B",
			"## Step 1",
			"Do something",
			"",
			"## Step 2",
		].join("\n");
		const result = recalculateThreadAnchors(
			threads,
			"workflow://plan",
			updated,
		);
		expect(result[0].lineNumber).toBe(5);
		expect(result[1].lineNumber).toBe(8);
	});
});
