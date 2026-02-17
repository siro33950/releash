import { diffLines } from "diff";

export type DiffRangeType = "added" | "modified";

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
