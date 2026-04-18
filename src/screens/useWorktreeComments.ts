import type { RefObject } from "react";
import { useCallback } from "react";
import type { TerminalTabPanelHandle } from "@/components/panels/TerminalTabPanel";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { trackEvent } from "@/lib/telemetry";
import type { Thread } from "@/types/thread";

interface UseWorktreeThreadsParams {
	terminalRef: RefObject<TerminalTabPanelHandle | null>;
	rootPath: string;
}

export function useWorktreeThreads({
	terminalRef,
	rootPath,
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

	const handleThreadClick = useCallback(
		(_threadFilePath: string, _lineNumber: number) => {
			// Editor is removed; thread-to-file navigation will be re-implemented in #785
		},
		[],
	);

	return {
		handleSendToTerminal,
		handleThreadClick,
	};
}
