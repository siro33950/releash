import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { createLineAnchor, recalculateThreadAnchors } from "@/lib/threadAnchor";
import type { Thread, ThreadEntry, ThreadSeverity } from "@/types/thread";

function makeEntry(
	content: string,
	isAi: boolean,
	authorName?: string,
	action?: "implement" | "posted-to-pr",
	prCommentId?: number,
): ThreadEntry {
	return {
		id: crypto.randomUUID(),
		content,
		isAi,
		...(authorName != null && { authorName }),
		...(action != null && { action }),
		...(prCommentId != null && { prCommentId }),
		createdAt: Date.now(),
	};
}

export function useThreads(worktreeName: string) {
	const [threads, setThreads] = useState<Thread[]>([]);
	const worktreeNameRef = useRef(worktreeName);
	worktreeNameRef.current = worktreeName;

	// Initial load — Rust now returns camelCase directly
	useEffect(() => {
		let disposed = false;
		setThreads([]);
		invoke<Thread[]>("load_threads", { worktreeName }).then(
			(loaded) => {
				if (!disposed) setThreads(loaded);
			},
			(err) => {
				if (!disposed) console.error("Failed to load threads:", err);
			},
		);
		return () => {
			disposed = true;
		};
	}, [worktreeName]);

	// Listen for external changes — payload now includes threads array
	useEffect(() => {
		const unlisten = listen<{
			worktree_name: string;
			source: string;
			threads: Thread[];
		}>("threads-changed", (event) => {
			if (event.payload.worktree_name !== worktreeNameRef.current) return;
			if (event.payload.source === "desktop") return;
			setThreads(event.payload.threads);
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	// Each mutation: invoke returns latest threads (no optimistic update)
	const createThread = useCallback(
		async (
			filePath: string,
			lineNumber: number,
			content: string,
			endLine?: number,
			severity?: ThreadSeverity,
			isAi?: boolean,
			authorName?: string,
			fileContent?: string,
		) => {
			const entry = makeEntry(content, isAi ?? false, authorName);
			const anchor =
				fileContent != null
					? createLineAnchor(fileContent, lineNumber)
					: undefined;
			const thread: Thread = {
				id: crypto.randomUUID(),
				filePath,
				lineNumber,
				...(endLine != null && { endLine }),
				entries: [entry],
				resolved: false,
				...(severity != null && { severity }),
				...(anchor != null && { anchor }),
				createdAt: Date.now(),
			};
			try {
				const latest = await invoke<Thread[]>("add_thread", {
					worktreeName: worktreeNameRef.current,
					thread,
					source: "desktop",
				});
				setThreads(latest);
			} catch (err) {
				console.error(err);
				throw err;
			}
			return thread;
		},
		[],
	);

	const addEntry = useCallback(
		async (
			threadId: string,
			content: string,
			isAi?: boolean,
			authorName?: string,
			action?: "implement" | "posted-to-pr",
			prCommentId?: number,
		) => {
			const entry = makeEntry(
				content,
				isAi ?? false,
				authorName,
				action,
				prCommentId,
			);
			try {
				const latest = await invoke<Thread[]>("add_thread_entry", {
					worktreeName: worktreeNameRef.current,
					threadId,
					entry,
					source: "desktop",
				});
				setThreads(latest);
			} catch (err) {
				console.error(err);
				throw err;
			}
			return entry;
		},
		[],
	);

	const removeThread = useCallback(async (threadId: string) => {
		try {
			const latest = await invoke<Thread[]>("remove_thread", {
				worktreeName: worktreeNameRef.current,
				threadId,
				source: "desktop",
			});
			setThreads(latest);
		} catch (err) {
			console.error(err);
		}
	}, []);

	const updateEntry = useCallback(
		async (threadId: string, entryId: string, content: string) => {
			try {
				const latest = await invoke<Thread[]>("update_thread_entry_content", {
					worktreeName: worktreeNameRef.current,
					threadId,
					entryId,
					content,
					source: "desktop",
				});
				setThreads(latest);
			} catch (err) {
				console.error(err);
			}
		},
		[],
	);

	const resolveThread = useCallback(async (threadId: string) => {
		try {
			const latest = await invoke<Thread[]>("toggle_resolve_thread", {
				worktreeName: worktreeNameRef.current,
				threadId,
				source: "desktop",
			});
			setThreads(latest);
		} catch (err) {
			console.error(err);
		}
	}, []);

	const getThreadsForFile = useCallback(
		(filePath: string) => {
			return threads.filter((t) => t.filePath === filePath);
		},
		[threads],
	);

	const recalculateAnchorsForFile = useCallback(
		(filePath: string, currentContent: string) => {
			const threadsToSync: Thread[] = [];
			setThreads((prev) => {
				const updated = recalculateThreadAnchors(
					prev,
					filePath,
					currentContent,
				);
				let changed = false;
				for (let i = 0; i < prev.length; i++) {
					if (prev[i] !== updated[i]) {
						changed = true;
						threadsToSync.push(updated[i]);
					}
				}
				return changed ? updated : prev;
			});
			for (const thread of threadsToSync) {
				invoke("update_thread", {
					worktreeName: worktreeNameRef.current,
					thread,
					source: "desktop",
				}).catch(console.error);
			}
		},
		[],
	);

	const [showResolvedThreads, setShowResolvedThreads] = useState(false);

	const toggleShowResolvedThreads = useCallback(() => {
		setShowResolvedThreads((prev) => !prev);
	}, []);

	const unresolvedThreads = threads.filter((t) => !t.resolved);

	return {
		threads,
		unresolvedThreads,
		createThread,
		addEntry,
		removeThread,
		updateEntry,
		resolveThread,
		getThreadsForFile,
		setThreads,
		recalculateAnchorsForFile,
		showResolvedThreads,
		toggleShowResolvedThreads,
	};
}
