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

export function getThreadInitialContent(
	thread: ReviewDiscussionThread,
): string {
	return thread.comments[0]?.content ?? "";
}

/**
 * Thread パネル等から「該当スレッド本体」へ遷移する際の情報。
 * - `lineNumber` がある: Diff ビュー内の該当スレッドブロックへスクロール
 * - `lineNumber` が無い ＝ ファイル/一般スレッド: ファイルを選択してポップオーバーで表示
 */
export type ThreadNavigationTarget = {
	filePath: string;
	threadId: string;
	lineNumber?: number;
	isFileComment: boolean;
};

export function toThreadNavigationTarget(
	thread: ReviewDiscussionThread,
): ThreadNavigationTarget {
	const lineNumber = getThreadLineNumber(thread);
	return {
		filePath: getThreadFilePath(thread),
		threadId: thread.id,
		lineNumber,
		isFileComment: lineNumber == null,
	};
}
