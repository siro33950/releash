import { useCallback, useEffect, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Thread } from "@/types/thread";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteThreadsOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
}

export function useRemoteThreads({ subscribe, send }: UseRemoteThreadsOptions) {
	const [threads, setThreads] = useState<Thread[]>([]);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "threads_sync") {
				// Rust now sends camelCase directly — no conversion needed
				setThreads(msg.payload.threads as Thread[]);
			}
		});
	}, [subscribe]);

	const createThread = useCallback(
		(
			filePath: string,
			lineNumber: number,
			content: string,
			endLine?: number,
		) => {
			send({
				type: "create_thread",
				payload: {
					file_path: filePath,
					line_number: lineNumber,
					...(endLine != null && { end_line: endLine }),
					content,
				},
			});
		},
		[send],
	);

	const addEntry = useCallback(
		(threadId: string, content: string) => {
			send({
				type: "add_thread_entry",
				payload: { thread_id: threadId, content },
			});
		},
		[send],
	);

	const resolveThread = useCallback(
		(threadId: string) => {
			send({
				type: "resolve_thread",
				payload: { thread_id: threadId, resolved: true },
			});
		},
		[send],
	);

	const deleteThread = useCallback(
		(threadId: string) => {
			send({
				type: "delete_thread",
				payload: { thread_id: threadId },
			});
		},
		[send],
	);

	const updateEntry = useCallback(
		(threadId: string, entryId: string, content: string) => {
			send({
				type: "update_thread_entry",
				payload: { thread_id: threadId, entry_id: entryId, content },
			});
		},
		[send],
	);

	return {
		threads,
		createThread,
		addEntry,
		resolveThread,
		deleteThread,
		updateEntry,
	};
}
