import type { LineComment } from "@/types/comment";

export function formatCommentsForTerminal(
	comments: LineComment[],
	rootPath?: string,
): string {
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

	const blocks: string[] = [];
	for (const [filePath, fileComments] of grouped) {
		const prefix = rootPath ? `${rootPath}/` : "";
		const relativePath = filePath.startsWith(prefix)
			? filePath.slice(prefix.length)
			: filePath;
		const sorted = [...fileComments].sort(
			(a, b) => a.lineNumber - b.lineNumber,
		);
		for (const c of sorted) {
			const lineLabel =
				c.endLine != null
					? `L${c.lineNumber}-${c.endLine}`
					: `L${c.lineNumber}`;
			blocks.push(`${relativePath}:${lineLabel}\n${c.content}`);
		}
	}

	return `## Review Comments\n${blocks.join("\n=====\n")}`;
}
