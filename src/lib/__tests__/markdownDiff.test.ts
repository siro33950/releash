import { describe, expect, it } from "vitest";
import {
	computeInlineChunks,
	computeModifiedDiffRanges,
	computeOriginalDiffRanges,
	computeSplitRows,
} from "../markdownDiff";

describe("computeModifiedDiffRanges", () => {
	it("returns empty array when contents are identical", () => {
		const text = "line1\nline2\nline3\n";
		expect(computeModifiedDiffRanges(text, text)).toEqual([]);
	});

	it("detects added lines", () => {
		const original = "line1\nline2\n";
		const modified = "line1\nline2\nline3\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 3, endLine: 3, type: "added" }]);
	});

	it("detects modified lines (removed + added adjacent)", () => {
		const original = "line1\nold line\nline3\n";
		const modified = "line1\nnew line\nline3\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 2, endLine: 2, type: "modified" }]);
	});

	it("handles removed-only lines (no range on modified side)", () => {
		const original = "line1\nremoved\nline3\n";
		const modified = "line1\nline3\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([]);
	});

	it("handles multiple added lines", () => {
		const original = "line1\n";
		const modified = "line1\nnew1\nnew2\nnew3\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 2, endLine: 4, type: "added" }]);
	});

	it("handles mixed changes", () => {
		const original = "a\nb\nc\nd\ne\n";
		const modified = "a\nB\nc\nnew\ne\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toContainEqual(
			expect.objectContaining({ type: "modified" }),
		);
	});

	it("handles completely new content (original empty)", () => {
		const original = "";
		const modified = "line1\nline2\n";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 1, endLine: 2, type: "added" }]);
	});

	it("handles modified becoming empty", () => {
		const original = "line1\nline2\n";
		const modified = "";
		const ranges = computeModifiedDiffRanges(original, modified);
		expect(ranges).toEqual([]);
	});
});

describe("computeOriginalDiffRanges", () => {
	it("returns empty array when contents are identical", () => {
		const text = "line1\nline2\n";
		expect(computeOriginalDiffRanges(text, text)).toEqual([]);
	});

	it("detects deleted lines on original side", () => {
		const original = "line1\nremoved\nline3\n";
		const modified = "line1\nline3\n";
		const ranges = computeOriginalDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 2, endLine: 2, type: "deleted" }]);
	});

	it("detects modified lines on original side", () => {
		const original = "line1\nold line\nline3\n";
		const modified = "line1\nnew line\nline3\n";
		const ranges = computeOriginalDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 2, endLine: 2, type: "modified" }]);
	});

	it("skips added-only lines (no range on original side)", () => {
		const original = "line1\nline2\n";
		const modified = "line1\nline2\nnew line\n";
		const ranges = computeOriginalDiffRanges(original, modified);
		expect(ranges).toEqual([]);
	});

	it("handles multiple deleted lines", () => {
		const original = "line1\ndel1\ndel2\ndel3\nline5\n";
		const modified = "line1\nline5\n";
		const ranges = computeOriginalDiffRanges(original, modified);
		expect(ranges).toEqual([{ startLine: 2, endLine: 4, type: "deleted" }]);
	});
});

describe("computeSplitRows", () => {
	it("returns single unchanged row when contents are identical", () => {
		const text = "hello world\n";
		const rows = computeSplitRows(text, text);
		expect(rows).toEqual([{ left: text, right: text, type: "unchanged" }]);
	});

	it("returns empty array when both are empty", () => {
		expect(computeSplitRows("", "")).toEqual([]);
	});

	it("detects all-new content as added", () => {
		const rows = computeSplitRows("", "new content\n");
		expect(rows).toEqual([
			{ left: null, right: "new content\n", type: "added" },
		]);
	});

	it("detects all-deleted content as removed", () => {
		const rows = computeSplitRows("old content\n", "");
		expect(rows).toEqual([
			{ left: "old content\n", right: null, type: "removed" },
		]);
	});

	it("detects modified (removed + added adjacent)", () => {
		const original = "line1\nold line\nline3\n";
		const modified = "line1\nnew line\nline3\n";
		const rows = computeSplitRows(original, modified);
		expect(rows).toHaveLength(3);
		expect(rows[0]).toEqual({
			left: "line1\n",
			right: "line1\n",
			type: "unchanged",
		});
		expect(rows[1]).toEqual({
			left: "old line\n",
			right: "new line\n",
			type: "modified",
		});
		expect(rows[2]).toEqual({
			left: "line3\n",
			right: "line3\n",
			type: "unchanged",
		});
	});

	it("detects removed-only (no adjacent added)", () => {
		const original = "line1\nremoved\nline3\n";
		const modified = "line1\nline3\n";
		const rows = computeSplitRows(original, modified);
		const removedRow = rows.find((r) => r.type === "removed");
		expect(removedRow).toEqual({
			left: "removed\n",
			right: null,
			type: "removed",
		});
	});

	it("detects added-only (no preceding removed)", () => {
		const original = "line1\nline2\n";
		const modified = "line1\nnew\nline2\n";
		const rows = computeSplitRows(original, modified);
		const addedRow = rows.find((r) => r.type === "added");
		expect(addedRow).toEqual({
			left: null,
			right: "new\n",
			type: "added",
		});
	});
});

describe("computeInlineChunks", () => {
	it("returns single unchanged chunk when contents are identical", () => {
		const text = "hello world\n";
		const chunks = computeInlineChunks(text, text);
		expect(chunks).toEqual([{ content: text, type: "unchanged" }]);
	});

	it("returns empty array when both are empty", () => {
		expect(computeInlineChunks("", "")).toEqual([]);
	});

	it("detects added chunk", () => {
		const chunks = computeInlineChunks("", "new content\n");
		expect(chunks).toEqual([{ content: "new content\n", type: "added" }]);
	});

	it("detects removed chunk", () => {
		const chunks = computeInlineChunks("old content\n", "");
		expect(chunks).toEqual([{ content: "old content\n", type: "removed" }]);
	});

	it("detects mixed changes", () => {
		const original = "line1\nold line\nline3\n";
		const modified = "line1\nnew line\nline3\n";
		const chunks = computeInlineChunks(original, modified);
		expect(chunks).toHaveLength(4);
		expect(chunks[0]).toEqual({ content: "line1\n", type: "unchanged" });
		expect(chunks[1]).toEqual({ content: "old line\n", type: "removed" });
		expect(chunks[2]).toEqual({ content: "new line\n", type: "added" });
		expect(chunks[3]).toEqual({ content: "line3\n", type: "unchanged" });
	});
});
