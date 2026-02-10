import { useCallback, useEffect, useState } from "react";
import type { LineComment } from "@/types/comment";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteContentOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
}

export function useRemoteContent({ subscribe, send }: UseRemoteContentOptions) {
	const [comments, setComments] = useState<LineComment[]>([]);
	const [branchName, setBranchName] = useState<string | null>(null);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "branch_info_response") {
				setBranchName(msg.payload.branch);
			}
			if (msg.type === "comments_sync") {
				setComments(
					msg.payload.comments.map((c) => ({
						id: c.id,
						filePath: c.file_path,
						lineNumber: c.line_number,
						...(c.end_line != null && { endLine: c.end_line }),
						content: c.content,
						status: c.status,
						createdAt: c.created_at,
					})),
				);
			}
		});
	}, [subscribe]);

	const addComment = useCallback(
		(
			filePath: string,
			lineNumber: number,
			content: string,
			endLine?: number,
		) => {
			send({
				type: "add_comment",
				payload: {
					file_path: filePath,
					line_number: lineNumber,
					...(endLine != null && { end_line: endLine }),
					content,
				},
			});
			const comment: LineComment = {
				id: `remote-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
				filePath,
				lineNumber,
				...(endLine != null && { endLine }),
				content,
				status: "unsent",
				createdAt: Date.now(),
			};
			setComments((prev) => [...prev, comment]);
		},
		[send],
	);

	return { comments, branchName, setComments, setBranchName, addComment };
}
