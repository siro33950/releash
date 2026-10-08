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
				hunkId: "h:added:0",
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
				hunkId: "h:deleted:0",
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
				hunkId: "h:file-deleted:0",
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
				hunkId: "h:no-newline:0",
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

	it("assigns change group id to matching change block by hunk offset", () => {
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
						hunkIndex: 0,
						hunkLineOffset: 0,
					},
				],
			},
		];
		const changeGroups = [
			{
				groupIndex: 0,
				groupId: "g:matching:0",
				hunkIndex: 0,
				newStart: 5,
				newEnd: 5,
				lineOffsetStart: 0,
				lineOffsetEnd: 0,
			},
		];
		const result = assignChangeGroupsToBlocks(blocks, changeGroups);
		expect(result[0].changeGroupId).toBe("g:matching:0");
	});

	it("splits one hunk change block into all matching change groups", () => {
		const hunks = [
			{
				index: 0,
				hunkId: "h:multi:0",
				oldStart: 1,
				oldLines: 5,
				newStart: 1,
				newLines: 5,
				lines: [
					" ctx1",
					"-old a",
					"+new a",
					" ctx2",
					"-old b",
					"+new b",
					" ctx3",
				],
			},
		];
		const { blocks } = computeDiffBlocks(
			hunks,
			null,
			null,
			"ctx1\nold a\nctx2\nold b\nctx3\n",
			"ctx1\nnew a\nctx2\nnew b\nctx3\n",
		);

		const result = assignChangeGroupsToBlocks(blocks, [
			{
				groupIndex: 0,
				groupId: "g:first",
				hunkIndex: 0,
				newStart: 2,
				newEnd: 2,
				lineOffsetStart: 1,
				lineOffsetEnd: 2,
			},
			{
				groupIndex: 1,
				groupId: "g:second",
				hunkIndex: 0,
				newStart: 4,
				newEnd: 4,
				lineOffsetStart: 4,
				lineOffsetEnd: 5,
			},
		]);

		const changeBlocks = result.filter((block) => block.type === "change");
		expect(changeBlocks).toHaveLength(2);
		expect(changeBlocks.map((block) => block.changeGroupId)).toEqual([
			"g:first",
			"g:second",
		]);
		expect(
			changeBlocks.map((block) => block.lines.map((line) => line.content)),
		).toEqual([
			["old a", "new a"],
			["old b", "new b"],
		]);
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
				hunkId: "h:fallback:0",
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
