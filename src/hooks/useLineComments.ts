import { useCallback, useState } from "react";
import type { LineComment } from "@/types/comment";

let nextId = 1;

export function useLineComments() {
	const [comments, setComments] = useState<LineComment[]>([]);

	const addComment = useCallback(
		(filePath: string, lineNumber: number, content: string, endLine?: number) => {
			const comment: LineComment = {
				id: `comment-${nextId++}`,
				filePath,
				lineNumber,
				...(endLine != null && { endLine }),
				content,
				status: "unsent",
				createdAt: Date.now(),
			};
			setComments((prev) => [...prev, comment]);
			return comment;
		},
		[],
	);

	const removeComment = useCallback((id: string) => {
		setComments((prev) => prev.filter((c) => c.id !== id));
	}, []);

	const updateComment = useCallback((id: string, content: string) => {
		setComments((prev) =>
			prev.map((c) => (c.id === id ? { ...c, content } : c)),
		);
	}, []);

	const markAsSent = useCallback((ids: string[]) => {
		const idSet = new Set(ids);
		setComments((prev) =>
			prev.map((c) => (idSet.has(c.id) ? { ...c, status: "sent" as const } : c)),
		);
	}, []);

	const getCommentsForFile = useCallback(
		(filePath: string) => {
			return comments.filter((c) => c.filePath === filePath);
		},
		[comments],
	);

	const unsentComments = comments.filter((c) => c.status === "unsent");

	return {
		comments,
		unsentComments,
		addComment,
		removeComment,
		updateComment,
		markAsSent,
		getCommentsForFile,
	};
}
