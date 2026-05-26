import type { ReviewThread } from "./protocol";

export type ReviewDiscussionThread = ReviewThread;

export function getThreadFilePath(thread: ReviewDiscussionThread): string {
	return thread.target.filePath ?? "";
}

export function getThreadLineNumber(
	thread: ReviewDiscussionThread,
): number | undefined {
	return thread.target.lineNumber ?? undefined;
}

export function getThreadEndLine(
	thread: ReviewDiscussionThread,
): number | undefined {
	return thread.target.endLine ?? undefined;
}

export function getThreadPreviewContent(
	thread: ReviewDiscussionThread,
): string {
	return thread.comments[thread.comments.length - 1]?.content ?? "";
}

export function getThreadInitialContent(
	thread: ReviewDiscussionThread,
): string {
	return thread.comments[0]?.content ?? "";
}
