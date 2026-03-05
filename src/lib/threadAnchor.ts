import type { LineAnchor, Thread } from "@/types/thread";

const CONTEXT_LINES = 3;
const SCORE_TARGET_EXACT = 3.0;
const SCORE_TARGET_TRIM = 1.5;
const SCORE_CONTEXT_EXACT = 1.0;
const SCORE_CONTEXT_TRIM = 0.5;
const SCORE_THRESHOLD = 0.3;

export function createLineAnchor(
	fileContent: string,
	lineNumber: number,
): LineAnchor {
	const lines = fileContent.split("\n");
	const idx = lineNumber - 1;
	const targetLine = lines[idx] ?? "";
	const startBefore = Math.max(0, idx - CONTEXT_LINES);
	const contextBefore = lines.slice(startBefore, idx);
	const contextAfter = lines.slice(idx + 1, idx + 1 + CONTEXT_LINES);

	return {
		targetLine,
		contextBefore,
		contextAfter,
		originalLineNumber: lineNumber,
	};
}

export function resolveAnchor(
	anchor: LineAnchor,
	currentContent: string,
): number | null {
	const lines = currentContent.split("\n");
	let bestScore = -1;
	let bestLine = -1;

	for (let i = 0; i < lines.length; i++) {
		let score = 0;

		if (lines[i] === anchor.targetLine) {
			score += SCORE_TARGET_EXACT;
		} else if (lines[i].trim() === anchor.targetLine.trim()) {
			score += SCORE_TARGET_TRIM;
		} else {
			continue;
		}

		for (let j = 0; j < anchor.contextBefore.length; j++) {
			const contextIdx = i - (anchor.contextBefore.length - j);
			if (contextIdx < 0 || contextIdx >= lines.length) continue;
			if (lines[contextIdx] === anchor.contextBefore[j]) {
				score += SCORE_CONTEXT_EXACT;
			} else if (lines[contextIdx].trim() === anchor.contextBefore[j].trim()) {
				score += SCORE_CONTEXT_TRIM;
			}
		}

		for (let j = 0; j < anchor.contextAfter.length; j++) {
			const contextIdx = i + 1 + j;
			if (contextIdx >= lines.length) continue;
			if (lines[contextIdx] === anchor.contextAfter[j]) {
				score += SCORE_CONTEXT_EXACT;
			} else if (lines[contextIdx].trim() === anchor.contextAfter[j].trim()) {
				score += SCORE_CONTEXT_TRIM;
			}
		}

		if (
			score > bestScore ||
			(score === bestScore &&
				Math.abs(i + 1 - anchor.originalLineNumber) <
					Math.abs(bestLine - anchor.originalLineNumber))
		) {
			bestScore = score;
			bestLine = i + 1;
		}
	}

	if (bestScore <= SCORE_THRESHOLD) return null;
	return bestLine;
}

export function recalculateThreadAnchors(
	threads: Thread[],
	filePath: string,
	currentContent: string,
): Thread[] {
	return threads.map((thread) => {
		if (thread.filePath !== filePath || !thread.anchor) return thread;

		const newLine = resolveAnchor(thread.anchor, currentContent);
		if (newLine === null || newLine === thread.lineNumber) return thread;

		return { ...thread, lineNumber: newLine };
	});
}
