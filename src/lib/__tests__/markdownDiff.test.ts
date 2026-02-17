import { describe, expect, it } from "vitest";
import { computeModifiedDiffRanges } from "../markdownDiff";

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
