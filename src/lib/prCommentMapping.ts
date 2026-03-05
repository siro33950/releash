import type { Thread, ThreadEntry } from "@/types/thread";

export interface PrReviewComment {
	id: number;
	path: string;
	line: number | null;
	original_line: number | null;
	body: string;
	author: {
		login: string;
		avatar_url: string | null;
	};
	in_reply_to_id: number | null;
	created_at: string;
}

/**
 * Convert PR review comments into Thread objects.
 *
 * Comments with `in_reply_to_id` are grouped as entries within the parent
 * comment's thread. Root comments (no `in_reply_to_id`) become new threads.
 */
export function prReviewCommentsToThreads(
	comments: PrReviewComment[],
): Thread[] {
	// Group: root comment id → list of replies
	const rootComments: PrReviewComment[] = [];
	const replies = new Map<number, PrReviewComment[]>();

	for (const comment of comments) {
		if (comment.in_reply_to_id != null) {
			const list = replies.get(comment.in_reply_to_id) ?? [];
			list.push(comment);
			replies.set(comment.in_reply_to_id, list);
		} else {
			rootComments.push(comment);
		}
	}

	return rootComments.map((root) => {
		const rootEntry = commentToEntry(root);
		const childEntries = (replies.get(root.id) ?? [])
			.sort(
				(a, b) =>
					new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
			)
			.map(commentToEntry);

		return {
			id: `pr-comment-${root.id}`,
			filePath: root.path,
			lineNumber: root.line ?? root.original_line ?? 1,
			entries: [rootEntry, ...childEntries],
			resolved: false,
			createdAt: new Date(root.created_at).getTime(),
		};
	});
}

function commentToEntry(comment: PrReviewComment): ThreadEntry {
	return {
		id: `pr-entry-${comment.id}`,
		content: comment.body,
		isAi: false,
		authorName: comment.author.login,
		...(comment.author.avatar_url != null && {
			authorAvatarUrl: comment.author.avatar_url,
		}),
		prCommentId: comment.id,
		createdAt: new Date(comment.created_at).getTime(),
	};
}
