import { describe, expect, it } from "vitest";
import { assignChangeGroupsToBlocks, computeDiffBlocks } from "./useDiffTokens";

describe("computeDiffBlocks", () => {
	it("returns context block for identical content", () => {
		const result = computeDiffBlocks(
			[],
			null,
			null,
			"line1\nline2\n",
			"line1\nline2\n",
		);
		expect(result.blocks).toHaveLength(1);
		expect(result.blocks[0].type).toBe("context");
		expect(result.blocks[0].lines).toHaveLength(2);
		expect(result.blocks[0].lines[0].type).toBe("context");
		expect(result.blocks[0].lines[0].content).toBe("line1");
		expect(result.blocks[0].lines[0].oldLineNumber).toBe(1);
		expect(result.blocks[0].lines[0].newLineNumber).toBe(1);
	});

	it("returns empty blocks for empty content", () => {
		const result = computeDiffBlocks([], null, null, "", "");
		expect(result.blocks).toHaveLength(0);
	});

	it("processes hunks with added lines", () => {
		const hunks = [
			{
				index: 0,
				oldStart: 1,
				oldLines: 2,
				newStart: 1,
				newLines: 3,
				lines: [" line1", "+added", " line2"],
			},
		];
		const result = computeDiffBlocks(
			hunks,
			null,
			null,
			"line1\nline2\n",
			"line1\nadded\nline2\n",
		);

		const changeBlock = result.blocks.find((b) => b.type === "change");
		if (changeBlock == null) throw new Error("change block not found");

		const addedLines = changeBlock.lines.filter((l) => l.type === "added");
		expect(addedLines).toHaveLength(1);
		expect(addedLines[0].content).toBe("added");
		expect(addedLines[0].newLineNumber).toBe(2);
		expect(addedLines[0].oldLineNumber).toBeNull();
	});

	it("processes hunks with deleted lines", () => {
		const hunks = [
			{
				index: 0,
				oldStart: 1,
				oldLines: 3,
				newStart: 1,
				newLines: 2,
				lines: [" line1", "-deleted", " line2"],
			},
		];
		const result = computeDiffBlocks(
			hunks,
			null,
			null,
			"line1\ndeleted\nline2\n",
			"line1\nline2\n",
		);

		const changeBlock = result.blocks.find((b) => b.type === "change");
		if (changeBlock == null) throw new Error("change block not found");

		const deletedLines = changeBlock.lines.filter((l) => l.type === "deleted");
		expect(deletedLines).toHaveLength(1);
		expect(deletedLines[0].content).toBe("deleted");
		expect(deletedLines[0].oldLineNumber).toBe(2);
		expect(deletedLines[0].newLineNumber).toBeNull();
	});

	it("applies token data from tokenized lines", () => {
		const modifiedTokens = [
			{ tokens: [{ content: "line1", color: "#ff0000", offset: 0 }] },
		];
		const result = computeDiffBlocks(
			[],
			null,
			modifiedTokens,
			"line1\n",
			"line1\n",
		);

		expect(result.blocks[0].lines[0].tokens).toHaveLength(1);
		expect(result.blocks[0].lines[0].tokens[0].content).toBe("line1");
		expect(result.blocks[0].lines[0].tokens[0].color).toBe("#ff0000");
	});

	it("processes all-deletion hunks (file deleted)", () => {
		// Simulates git2 output for "content\n" → "" (file deleted)
		// git2 returns: old_start=1, old_lines=1, new_start=0, new_lines=0
		const hunks = [
			{
				index: 0,
				oldStart: 1,
				oldLines: 1,
				newStart: 0,
				newLines: 0,
				lines: ["-content"],
			},
		];
		const result = computeDiffBlocks(hunks, null, null, "content\n", "");

		expect(result.blocks).toHaveLength(1);
		expect(result.blocks[0].type).toBe("change");
		expect(result.blocks[0].lines).toHaveLength(1);
		expect(result.blocks[0].lines[0].type).toBe("deleted");
		expect(result.blocks[0].lines[0].content).toBe("content");
		expect(result.blocks[0].lines[0].oldLineNumber).toBe(1);
		expect(result.blocks[0].lines[0].newLineNumber).toBeNull();
	});

	it("skips backslash-only lines in hunks", () => {
		const hunks = [
			{
				index: 0,
				oldStart: 1,
				oldLines: 1,
				newStart: 1,
				newLines: 1,
				lines: ["-old", "+new", "\\ No newline at end of file"],
			},
		];
		const result = computeDiffBlocks(hunks, null, null, "old", "new");

		const changeBlock = result.blocks.find((b) => b.type === "change");
		if (changeBlock == null) throw new Error("change block not found");
		expect(
			changeBlock.lines.some((l) => l.content.includes("No newline")),
		).toBe(false);
	});
});

describe("assignChangeGroupsToBlocks", () => {
	it("returns blocks unchanged when no change groups", () => {
		const blocks = [
			{
				type: "context" as const,
				lines: [
					{
						type: "context" as const,
						oldLineNumber: 1,
						newLineNumber: 1,
						tokens: [],
						content: "line1",
					},
				],
			},
		];
		const result = assignChangeGroupsToBlocks(blocks, []);
		expect(result).toEqual(blocks);
	});

	it("assigns change group index to matching change block", () => {
		const blocks = [
			{
				type: "change" as const,
				lines: [
					{
						type: "added" as const,
						oldLineNumber: null,
						newLineNumber: 5,
						tokens: [],
						content: "new line",
					},
				],
			},
		];
		const changeGroups = [{ groupIndex: 0, newStart: 5, newEnd: 5 }];
		const result = assignChangeGroupsToBlocks(blocks, changeGroups);
		expect(result[0].changeGroupIndex).toBe(0);
	});
});

describe("progressive rendering fallback", () => {
	it("returns content as plain token when tokens are null", () => {
		const result = computeDiffBlocks(
			[],
			null,
			null,
			"hello world\n",
			"hello world\n",
		);
		expect(result.blocks).toHaveLength(1);
		expect(result.blocks[0].lines[0].tokens).toHaveLength(1);
		expect(result.blocks[0].lines[0].tokens[0].content).toBe("hello world");
	});

	it("returns shiki tokens when tokenized lines are provided", () => {
		const modifiedTokens = [
			{ tokens: [{ content: "hello", color: "#ff0000", offset: 0 }] },
		];
		const result = computeDiffBlocks(
			[],
			null,
			modifiedTokens,
			"hello\n",
			"hello\n",
		);
		expect(result.blocks[0].lines[0].tokens).toHaveLength(1);
		expect(result.blocks[0].lines[0].tokens[0].content).toBe("hello");
		expect(result.blocks[0].lines[0].tokens[0].color).toBe("#ff0000");
	});

	it("returns fallback tokens for hunk lines when tokens are null", () => {
		const hunks = [
			{
				index: 0,
				oldStart: 1,
				oldLines: 1,
				newStart: 1,
				newLines: 2,
				lines: [" ctx", "+added"],
			},
		];
		const result = computeDiffBlocks(
			hunks,
			null,
			null,
			"ctx\n",
			"ctx\nadded\n",
		);
		const changeBlock = result.blocks.find((b) => b.type === "change");
		if (changeBlock == null) throw new Error("change block not found");

		const addedLine = changeBlock.lines.find((l) => l.type === "added");
		expect(addedLine).toBeDefined();
		expect(addedLine?.tokens).toHaveLength(1);
		expect(addedLine?.tokens[0].content).toBe("added");
	});
});
