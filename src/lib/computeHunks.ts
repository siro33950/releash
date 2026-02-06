import { structuredPatch } from "diff";

export interface Hunk {
	index: number;
	oldStart: number;
	oldLines: number;
	newStart: number;
	newLines: number;
	lines: string[];
}

export interface ChangeGroup {
	groupIndex: number;
	hunkIndex: number;
	newStart: number;
	newEnd: number;
	lineOffsetStart: number;
	lineOffsetEnd: number;
	isStaged?: boolean;
}

export function computeHunks(
	original: string,
	modified: string,
	filePath = "file",
): Hunk[] {
	if (typeof original !== "string" || typeof modified !== "string") {
		return [];
	}
	const patch = structuredPatch(filePath, filePath, original, modified);
	return patch.hunks.map((h, i) => ({
		index: i,
		oldStart: h.oldStart,
		oldLines: h.oldLines,
		newStart: h.newStart,
		newLines: h.newLines,
		lines: h.lines,
	}));
}

export function splitHunkIntoGroups(
	hunk: Hunk,
	startGroupIndex: number,
): ChangeGroup[] {
	const groups: ChangeGroup[] = [];
	let modifiedLine = hunk.newStart;
	let inGroup = false;
	let groupStartOffset = 0;
	let groupNewStart = 0;
	let lastPlusLine = 0;
	let hasPlus = false;

	for (let i = 0; i < hunk.lines.length; i++) {
		const prefix = hunk.lines[i][0];

		if (prefix === "+" || prefix === "-") {
			if (!inGroup) {
				inGroup = true;
				groupStartOffset = i;
				groupNewStart = modifiedLine;
				lastPlusLine = 0;
				hasPlus = false;
			}
			if (prefix === "+") {
				lastPlusLine = modifiedLine;
				hasPlus = true;
				modifiedLine++;
			}
		} else {
			if (inGroup) {
				const newEnd = hasPlus
					? lastPlusLine
					: Math.max(groupNewStart - 1, 1);
				groups.push({
					groupIndex: startGroupIndex + groups.length,
					hunkIndex: hunk.index,
					newStart: hasPlus
						? groupNewStart
						: Math.max(groupNewStart - 1, 1),
					newEnd,
					lineOffsetStart: groupStartOffset,
					lineOffsetEnd: i - 1,
				});
				inGroup = false;
			}
			modifiedLine++;
		}
	}

	if (inGroup) {
		const newEnd = hasPlus
			? lastPlusLine
			: Math.max(groupNewStart - 1, 1);
		groups.push({
			groupIndex: startGroupIndex + groups.length,
			hunkIndex: hunk.index,
			newStart: hasPlus ? groupNewStart : Math.max(groupNewStart - 1, 1),
			newEnd,
			lineOffsetStart: groupStartOffset,
			lineOffsetEnd: hunk.lines.length - 1,
		});
	}

	return groups;
}

export function computeChangeGroups(hunks: Hunk[]): ChangeGroup[] {
	const groups: ChangeGroup[] = [];
	for (const hunk of hunks) {
		groups.push(...splitHunkIntoGroups(hunk, groups.length));
	}
	return groups;
}

function extractGroupLines(group: ChangeGroup, hunks: Hunk[]): string[] {
	const hunk = hunks.find((h) => h.index === group.hunkIndex);
	if (!hunk) return [];
	return hunk.lines.slice(group.lineOffsetStart, group.lineOffsetEnd + 1);
}

function getGroupOldPosition(group: ChangeGroup, hunks: Hunk[]): number {
	const hunk = hunks.find((h) => h.index === group.hunkIndex);
	if (!hunk) return -1;
	let oldLine = hunk.oldStart;
	for (let i = 0; i < group.lineOffsetStart; i++) {
		const prefix = hunk.lines[i]?.[0];
		if (prefix === "-" || prefix === " ") oldLine++;
	}
	return oldLine;
}

export function markStagedGroups(
	groups: ChangeGroup[],
	stagedGroups: ChangeGroup[],
	hunks: Hunk[],
	stagedHunks: Hunk[],
): ChangeGroup[] {
	const stagedKeysSet = new Set<string>();
	for (const sg of stagedGroups) {
		const lines = extractGroupLines(sg, stagedHunks);
		const pos = getGroupOldPosition(sg, stagedHunks);
		stagedKeysSet.add(`${pos}:${lines.join("\n")}`);
	}

	return groups.map((g) => {
		const lines = extractGroupLines(g, hunks);
		const pos = getGroupOldPosition(g, hunks);
		const key = `${pos}:${lines.join("\n")}`;
		return { ...g, isStaged: stagedKeysSet.has(key) };
	});
}
