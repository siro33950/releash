import type { TokenizedLine } from "@/hooks/useShikiHighlighter";
import type { Hunk } from "@/lib/computeHunks";

type DiffLineType = "added" | "deleted" | "context";

export interface DiffLine {
	type: DiffLineType;
	oldLineNumber: number | null;
	newLineNumber: number | null;
	tokens: TokenizedLine["tokens"];
	content: string;
	hunkIndex?: number;
	hunkLineOffset?: number;
}

export interface DiffBlock {
	type: "change" | "context";
	lines: DiffLine[];
	changeGroupId?: string;
}

function splitContentToLines(content: string): string[] {
	if (content === "") return [];
	const lines = content.split("\n");
	if (lines.length > 0 && lines[lines.length - 1] === "") {
		lines.pop();
	}
	return lines;
}

function getTokensForLine(
	tokenizedLines: TokenizedLine[] | null,
	lineIndex: number,
	fallbackContent?: string,
): TokenizedLine["tokens"] {
	if (tokenizedLines && lineIndex >= 0 && lineIndex < tokenizedLines.length) {
		return tokenizedLines[lineIndex].tokens;
	}
	if (fallbackContent !== undefined) {
		return [{ content: fallbackContent, color: undefined, offset: 0 }];
	}
	return [];
}

export interface DiffTokensResult {
	blocks: DiffBlock[];
	originalLines: string[];
	modifiedLines: string[];
}

export function computeDiffBlocks(
	hunks: Hunk[],
	originalTokens: TokenizedLine[] | null,
	modifiedTokens: TokenizedLine[] | null,
	originalContent: string,
	modifiedContent: string,
): DiffTokensResult {
	const originalLines = splitContentToLines(originalContent);
	const modifiedLines = splitContentToLines(modifiedContent);

	if (hunks.length === 0) {
		const lines: DiffLine[] = modifiedLines.map((line, i) => ({
			type: "context" as const,
			oldLineNumber: i + 1,
			newLineNumber: i + 1,
			tokens: getTokensForLine(modifiedTokens, i, line),
			content: line,
		}));
		return {
			blocks: lines.length > 0 ? [{ type: "context", lines }] : [],
			originalLines,
			modifiedLines,
		};
	}

	const blocks: DiffBlock[] = [];
	let currentOldLine = 1;
	let currentNewLine = 1;

	for (const hunk of hunks) {
		if (currentNewLine < hunk.newStart) {
			const contextLines: DiffLine[] = [];
			while (currentNewLine < hunk.newStart) {
				const oldIdx = currentOldLine - 1;
				const newIdx = currentNewLine - 1;
				const lineContent =
					modifiedLines[newIdx] ?? originalLines[oldIdx] ?? "";
				contextLines.push({
					type: "context",
					oldLineNumber: currentOldLine,
					newLineNumber: currentNewLine,
					tokens: getTokensForLine(modifiedTokens, newIdx, lineContent),
					content: lineContent,
				});
				currentOldLine++;
				currentNewLine++;
			}
			if (contextLines.length > 0) {
				blocks.push({ type: "context", lines: contextLines });
			}
		}

		const changeLines: DiffLine[] = [];
		for (
			let hunkLineOffset = 0;
			hunkLineOffset < hunk.lines.length;
			hunkLineOffset++
		) {
			const rawLine = hunk.lines[hunkLineOffset];
			const prefix = rawLine[0];
			const content = rawLine.slice(1);

			if (prefix === "\\") continue;

			if (prefix === "-") {
				const oldIdx = currentOldLine - 1;
				changeLines.push({
					type: "deleted",
					oldLineNumber: currentOldLine,
					newLineNumber: null,
					tokens: getTokensForLine(originalTokens, oldIdx, content),
					content,
					hunkIndex: hunk.index,
					hunkLineOffset,
				});
				currentOldLine++;
			} else if (prefix === "+") {
				const newIdx = currentNewLine - 1;
				changeLines.push({
					type: "added",
					oldLineNumber: null,
					newLineNumber: currentNewLine,
					tokens: getTokensForLine(modifiedTokens, newIdx, content),
					content,
					hunkIndex: hunk.index,
					hunkLineOffset,
				});
				currentNewLine++;
			} else {
				const newIdx = currentNewLine - 1;
				changeLines.push({
					type: "context",
					oldLineNumber: currentOldLine,
					newLineNumber: currentNewLine,
					tokens: getTokensForLine(modifiedTokens, newIdx, content),
					content,
					hunkIndex: hunk.index,
					hunkLineOffset,
				});
				currentOldLine++;
				currentNewLine++;
			}
		}

		if (changeLines.length > 0) {
			blocks.push({ type: "change", lines: changeLines });
		}
	}

	if (currentNewLine <= modifiedLines.length) {
		const contextLines: DiffLine[] = [];
		while (currentNewLine <= modifiedLines.length) {
			const newIdx = currentNewLine - 1;
			const lineContent = modifiedLines[newIdx] ?? "";
			contextLines.push({
				type: "context",
				oldLineNumber: currentOldLine,
				newLineNumber: currentNewLine,
				tokens: getTokensForLine(modifiedTokens, newIdx, lineContent),
				content: lineContent,
			});
			currentOldLine++;
			currentNewLine++;
		}
		if (contextLines.length > 0) {
			blocks.push({ type: "context", lines: contextLines });
		}
	}

	return { blocks, originalLines, modifiedLines };
}

export function assignChangeGroupsToBlocks(
	blocks: DiffBlock[],
	changeGroups: {
		groupIndex: number;
		groupId: string;
		hunkIndex: number;
		newStart: number;
		newEnd: number;
		lineOffsetStart: number;
		lineOffsetEnd: number;
	}[],
): DiffBlock[] {
	if (changeGroups.length === 0) return blocks;

	return blocks.flatMap((block) => {
		if (block.type !== "change") return block;

		const lineGroups = block.lines.map((line) =>
			changeGroups.find(
				(cg) =>
					line.hunkIndex === cg.hunkIndex &&
					line.hunkLineOffset != null &&
					line.hunkLineOffset >= cg.lineOffsetStart &&
					line.hunkLineOffset <= cg.lineOffsetEnd,
			),
		);

		if (lineGroups.some((group) => group != null)) {
			const result: DiffBlock[] = [];
			let currentLines: DiffLine[] = [];
			let currentGroup = lineGroups[0];
			const flush = () => {
				if (currentLines.length === 0) return;
				if (currentGroup != null) {
					result.push({
						type: "change",
						lines: currentLines,
						changeGroupId: currentGroup.groupId,
					});
				} else {
					result.push({
						type: currentLines.some((line) => line.type !== "context")
							? "change"
							: "context",
						lines: currentLines,
					});
				}
				currentLines = [];
			};

			for (let i = 0; i < block.lines.length; i++) {
				const group = lineGroups[i];
				if (group?.groupId !== currentGroup?.groupId) {
					flush();
					currentGroup = group;
				}
				currentLines.push(block.lines[i]);
			}
			flush();
			return result;
		}

		return block;
	});
}
