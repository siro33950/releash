import type { ChangeGroup, Hunk } from "./computeHunks";

export function generatePatch(
	filePath: string,
	hunks: Hunk[],
	selectedIndices: number[],
): string {
	const selected = hunks.filter((h) => selectedIndices.includes(h.index));
	if (selected.length === 0) return "";

	const lines: string[] = [];
	lines.push(`--- a/${filePath}`);
	lines.push(`+++ b/${filePath}`);

	for (const hunk of selected) {
		lines.push(
			`@@ -${hunk.oldStart},${hunk.oldLines} +${hunk.newStart},${hunk.newLines} @@`,
		);
		for (const line of hunk.lines) {
			lines.push(line);
		}
	}

	return `${lines.join("\n")}\n`;
}

export function generateGroupPatch(
	filePath: string,
	hunk: Hunk,
	group: ChangeGroup,
): string {
	const resultLines: string[] = [];

	for (let i = 0; i < hunk.lines.length; i++) {
		const line = hunk.lines[i];
		const prefix = line[0];

		if (i >= group.lineOffsetStart && i <= group.lineOffsetEnd) {
			resultLines.push(line);
		} else if (prefix === "-") {
			resultLines.push(` ${line.slice(1)}`);
		} else if (prefix === "+") {
			continue;
		} else {
			resultLines.push(line);
		}
	}

	let oldLines = 0;
	let newLines = 0;
	for (const line of resultLines) {
		const p = line[0];
		if (p === " ") {
			oldLines++;
			newLines++;
		} else if (p === "-") {
			oldLines++;
		} else if (p === "+") {
			newLines++;
		}
	}

	const lines: string[] = [];
	lines.push(`--- a/${filePath}`);
	lines.push(`+++ b/${filePath}`);
	lines.push(
		`@@ -${hunk.oldStart},${oldLines} +${hunk.oldStart},${newLines} @@`,
	);
	for (const line of resultLines) {
		lines.push(line);
	}

	return `${lines.join("\n")}\n`;
}
