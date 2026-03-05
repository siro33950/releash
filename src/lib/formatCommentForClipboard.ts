import type { Thread } from "@/types/thread";

export function formatCommentForClipboard(thread: Thread): string {
	const lineLabel =
		thread.endLine != null
			? `L${thread.lineNumber}-${thread.endLine}`
			: `L${thread.lineNumber}`;
	const firstContent = thread.entries[0]?.content ?? "";
	return `${thread.filePath}:${lineLabel}\n${firstContent}`;
}
