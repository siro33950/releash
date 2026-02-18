import { diffLines } from "diff";

export type DiffRangeType = "added" | "modified" | "deleted";

export interface DiffRange {
	startLine: number; // 1-based
	endLine: number; // 1-based, inclusive
	type: DiffRangeType;
}

export function computeModifiedDiffRanges(
	original: string,
	modified: string,
): DiffRange[] {
	if (original === modified) return [];

	const changes = diffLines(original, modified);
	const ranges: DiffRange[] = [];
	let modifiedLine = 1;

	for (let i = 0; i < changes.length; i++) {
		const change = changes[i];
		const lines = change.count ?? 0;

		if (change.removed) {
			const next = changes[i + 1];
			if (next?.added) {
				// removed + added adjacent → modified
				const addedLines = next.count ?? 0;
				ranges.push({
					startLine: modifiedLine,
					endLine: modifiedLine + addedLines - 1,
					type: "modified",
				});
				modifiedLine += addedLines;
				i++; // skip the added part
			}
			// removed only → no range on modified side
		} else if (change.added) {
			ranges.push({
				startLine: modifiedLine,
				endLine: modifiedLine + lines - 1,
				type: "added",
			});
			modifiedLine += lines;
		} else {
			modifiedLine += lines;
		}
	}

	return ranges;
}

export function computeOriginalDiffRanges(
	original: string,
	modified: string,
): DiffRange[] {
	if (original === modified) return [];

	const changes = diffLines(original, modified);
	const ranges: DiffRange[] = [];
	let originalLine = 1;

	for (let i = 0; i < changes.length; i++) {
		const change = changes[i];
		const lines = change.count ?? 0;

		if (change.removed) {
			const next = changes[i + 1];
			if (next?.added) {
				// removed + added adjacent → modified (original side range)
				ranges.push({
					startLine: originalLine,
					endLine: originalLine + lines - 1,
					type: "modified",
				});
				originalLine += lines;
				i++; // skip the added part
			} else {
				// removed only → deleted on original side
				ranges.push({
					startLine: originalLine,
					endLine: originalLine + lines - 1,
					type: "deleted",
				});
				originalLine += lines;
			}
		} else if (change.added) {
			// added only → no range on original side
		} else {
			originalLine += lines;
		}
	}

	return ranges;
}

export interface SplitRow {
	left: string | null;
	right: string | null;
	type: "unchanged" | "added" | "removed" | "modified";
}

export function computeSplitRows(
	original: string,
	modified: string,
): SplitRow[] {
	if (original === modified) {
		return original
			? [{ left: original, right: original, type: "unchanged" }]
			: [];
	}

	const changes = diffLines(original, modified);
	const rows: SplitRow[] = [];

	for (let i = 0; i < changes.length; i++) {
		const change = changes[i];
		if (!change.value) continue;

		if (change.removed) {
			const next = changes[i + 1];
			if (next?.added) {
				rows.push({
					left: change.value,
					right: next.value,
					type: "modified",
				});
				i++;
			} else {
				rows.push({ left: change.value, right: null, type: "removed" });
			}
		} else if (change.added) {
			rows.push({ left: null, right: change.value, type: "added" });
		} else {
			rows.push({
				left: change.value,
				right: change.value,
				type: "unchanged",
			});
		}
	}

	return rows;
}

export interface InlineChunk {
	content: string;
	type: "unchanged" | "added" | "removed";
}

export function computeInlineChunks(
	original: string,
	modified: string,
): InlineChunk[] {
	if (original === modified) {
		return original ? [{ content: original, type: "unchanged" }] : [];
	}

	const changes = diffLines(original, modified);
	const chunks: InlineChunk[] = [];

	for (const change of changes) {
		if (!change.value) continue;
		if (change.removed) {
			chunks.push({ content: change.value, type: "removed" });
		} else if (change.added) {
			chunks.push({ content: change.value, type: "added" });
		} else {
			chunks.push({ content: change.value, type: "unchanged" });
		}
	}

	return chunks;
}
