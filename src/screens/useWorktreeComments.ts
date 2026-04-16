import type { RefObject } from "react";
import { useCallback } from "react";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { trackEvent } from "@/lib/telemetry";
import type { Thread } from "@/types/thread";
import type { EditorAction } from "./useWorktreeGitActions";

interface UseWorktreeThreadsParams {
	activeTabPath: string | null;
	handleOpenFile: (path: string) => Promise<void>;
	terminalRef: RefObject<TerminalTabPanelHandle | null>;
	rootPath: string;
	dispatchEditor: React.Dispatch<EditorAction>;
}

export function useWorktreeThreads({
	activeTabPath,
	handleOpenFile,
	terminalRef,
	rootPath,
	dispatchEditor,
}: UseWorktreeThreadsParams) {
	const handleSendToTerminal = useCallback(
		(threadsToSend: Thread[]) => {
			const text = formatCommentsForTerminal(threadsToSend, rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(text);
				terminalRef.current.writeToTerminal("\r");
				trackEvent("comment_sent", { count: threadsToSend.length });
			}
		},
		[rootPath, terminalRef],
	);

	const handleSendThread = useCallback(
		(thread: Thread) => {
			const text = formatCommentsForTerminal([thread], rootPath);
			if (text && terminalRef.current) {
				terminalRef.current.writeToTerminal(text);
				terminalRef.current.writeToTerminal("\r");
				trackEvent("comment_sent", { count: 1 });
			}
		},
		[rootPath, terminalRef],
	);

	const handleCopyThread = useCallback((thread: Thread) => {
		const text = formatCommentForClipboard(thread);
		navigator.clipboard.writeText(text).catch(() => {});
		trackEvent("comment_copied");
	}, []);

	const handleThreadClick = useCallback(
		(threadFilePath: string, lineNumber: number) => {
			const absolutePath = threadFilePath.startsWith("/")
				? threadFilePath
				: `${rootPath}/${threadFilePath}`;
			if (activeTabPath === absolutePath) {
				dispatchEditor({
					type: "SET_PENDING_REVEAL",
					reveal: { path: absolutePath, line: lineNumber, openThread: true },
				});
			} else {
				handleOpenFile(absolutePath);
				dispatchEditor({
					type: "SET_PENDING_REVEAL",
					reveal: { path: absolutePath, line: lineNumber, openThread: true },
				});
			}
		},
		[activeTabPath, handleOpenFile, dispatchEditor, rootPath],
	);

	return {
		handleSendToTerminal,
		handleSendThread,
		handleCopyThread,
		handleThreadClick,
	};
}
