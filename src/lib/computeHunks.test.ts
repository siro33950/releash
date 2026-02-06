import { describe, expect, it } from "vitest";
import {
	computeChangeGroups,
	computeHunks,
	markStagedGroups,
} from "./computeHunks";

describe("computeHunks", () => {
	it("should return empty array for identical content", () => {
		const hunks = computeHunks("hello\nworld\n", "hello\nworld\n");
		expect(hunks).toEqual([]);
	});

	it("should detect added lines", () => {
		const hunks = computeHunks("line1\nline2\n", "line1\nline2\nline3\n");
		expect(hunks.length).toBe(1);
		expect(hunks[0].lines).toContain("+line3");
	});

	it("should detect removed lines", () => {
		const hunks = computeHunks("line1\nline2\nline3\n", "line1\nline3\n");
		expect(hunks.length).toBe(1);
		expect(hunks[0].lines).toContain("-line2");
	});

	it("should detect modified lines", () => {
		const hunks = computeHunks(
			"line1\noriginal\nline3\n",
			"line1\nmodified\nline3\n",
		);
		expect(hunks.length).toBe(1);
		expect(hunks[0].lines).toContain("-original");
		expect(hunks[0].lines).toContain("+modified");
	});

	it("should detect multiple hunks", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const hunks = computeHunks(lines, modified);
		expect(hunks.length).toBe(2);
	});

	it("should handle empty original", () => {
		const hunks = computeHunks("", "new content\n");
		expect(hunks.length).toBe(1);
		expect(hunks[0].lines).toContain("+new content");
	});

	it("should handle empty modified", () => {
		const hunks = computeHunks("content\n", "");
		expect(hunks.length).toBe(1);
		expect(hunks[0].lines).toContain("-content");
	});

	it("should assign sequential indices", () => {
		const lines =
			"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";
		const modified = lines.replace("b\n", "B\n").replace("r\n", "R\n");
		const hunks = computeHunks(lines, modified);
		expect(hunks[0].index).toBe(0);
		expect(hunks[1].index).toBe(1);
	});

	it("should include oldStart/newStart/oldLines/newLines", () => {
		const hunks = computeHunks("line1\nline2\n", "line1\nline2\nline3\n");
		expect(hunks[0]).toHaveProperty("oldStart");
		expect(hunks[0]).toHaveProperty("newStart");
		expect(hunks[0]).toHaveProperty("oldLines");
		expect(hunks[0]).toHaveProperty("newLines");
	});
});

describe("markStagedGroups", () => {
	const head = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\no\np\nq\nr\ns\nt\n";

	it("should mark groups as staged when staged diff matches", () => {
		const working = head.replace("b\n", "B\n").replace("r\n", "R\n");
		const staged = head.replace("b\n", "B\n");

		const hunks = computeHunks(head, working);
		const groups = computeChangeGroups(hunks);
		const stagedHunks = computeHunks(head, staged);
		const stagedGroups = computeChangeGroups(stagedHunks);

		const result = markStagedGroups(groups, stagedGroups, hunks, stagedHunks);

		expect(result).toHaveLength(2);
		expect(result[0].isStaged).toBe(true);
		expect(result[1].isStaged).toBe(false);
	});

	it("should mark all groups as staged when all changes are staged", () => {
		const working = head.replace("b\n", "B\n");
		const staged = working;

		const hunks = computeHunks(head, working);
		const groups = computeChangeGroups(hunks);
		const stagedHunks = computeHunks(head, staged);
		const stagedGroups = computeChangeGroups(stagedHunks);

		const result = markStagedGroups(groups, stagedGroups, hunks, stagedHunks);

		expect(result).toHaveLength(1);
		expect(result[0].isStaged).toBe(true);
	});

	it("should mark all groups as unstaged when nothing is staged", () => {
		const working = head.replace("b\n", "B\n");

		const hunks = computeHunks(head, working);
		const groups = computeChangeGroups(hunks);
		const stagedHunks = computeHunks(head, head);
		const stagedGroups = computeChangeGroups(stagedHunks);

		const result = markStagedGroups(groups, stagedGroups, hunks, stagedHunks);

		expect(result).toHaveLength(1);
		expect(result[0].isStaged).toBe(false);
	});

	it("should handle empty groups", () => {
		const result = markStagedGroups([], [], [], []);
		expect(result).toEqual([]);
	});
});
