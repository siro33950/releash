import type { LineComment } from "@/types/comment";

export function formatCommentForClipboard(comment: LineComment): string {
	const lineLabel =
		comment.endLine != null
			? `L${comment.lineNumber}-${comment.endLine}`
			: `L${comment.lineNumber}`;
	return `${comment.filePath}:${lineLabel}\n${comment.content}`;
}
