export type ThreadSeverity = "info" | "warning" | "error" | "suggestion";

export type ThreadOrigin = "local" | "ai-review" | "pr";

export interface LineAnchor {
	targetLine: string;
	contextBefore: string[];
	contextAfter: string[];
	originalLineNumber: number;
}

export interface ThreadEntry {
	id: string;
	content: string;
	isAi: boolean;
	action?: "implement" | "posted-to-pr";
	authorName?: string;
	authorAvatarUrl?: string;
	prCommentId?: number;
	createdAt: number;
}

export interface Thread {
	id: string;
	filePath: string;
	lineNumber: number;
	endLine?: number;
	entries: ThreadEntry[];
	resolved: boolean;
	severity?: ThreadSeverity;
	anchor?: LineAnchor;
	createdAt: number;
}

export function getThreadOrigin(thread: Thread): ThreadOrigin {
	const first = thread.entries[0];
	if (!first) return "local";
	if (first.prCommentId != null) return "pr";
	if (first.isAi) return "ai-review";
	return "local";
}
