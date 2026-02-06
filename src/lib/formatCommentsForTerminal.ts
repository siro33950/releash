import type { LineComment } from "@/types/comment";

export function formatCommentsForTerminal(comments: LineComment[]): string {
	if (comments.length === 0) return "";

	const grouped = new Map<string, LineComment[]>();
	for (const comment of comments) {
		const existing = grouped.get(comment.filePath);
		if (existing) {
			existing.push(comment);
		} else {
			grouped.set(comment.filePath, [comment]);
		}
	}

	const lines: string[] = ["## Review Comments"];
	for (const [filePath, fileComments] of grouped) {
		const name = filePath.split("/").pop() ?? filePath;
		lines.push(`### ${name}`);
		const sorted = [...fileComments].sort(
			(a, b) => a.lineNumber - b.lineNumber,
		);
		for (const c of sorted) {
			const lineLabel = c.endLine != null
				? `L${c.lineNumber}-${c.endLine}`
				: `L${c.lineNumber}`;
			lines.push(`- ${lineLabel}: ${c.content}`);
		}
	}

	return lines.join("\n");
}
