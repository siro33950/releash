import { useCallback } from "react";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import type { LineComment } from "@/types/comment";
import type { WsMessage } from "@/types/protocol";
import type { Thread } from "@/types/thread";
import type { Tab } from "./useRemoteNavigation";

function lineCommentToThread(c: LineComment): Thread {
	return {
		id: c.id,
		filePath: c.filePath,
		lineNumber: c.lineNumber,
		...(c.endLine != null && { endLine: c.endLine }),
		entries: [
			{
				id: c.id,
				content: c.content,
				createdAt: c.createdAt,
			},
		],
		resolved: c.resolved,
		...(c.severity != null && { severity: c.severity }),
		createdAt: c.createdAt,
	};
}

interface UseRemoteAppActionsParams {
	send: (msg: WsMessage) => void;
	disconnect: () => void;
	setConnection: (value: { url: string; token: string } | null) => void;

	activePtyId: number | null;

	setSelectedWorktree: (worktree: string | null) => void;
	setActiveTab: (tab: Tab) => void;
	setTerminalMounted: (mounted: boolean) => void;
	setComments: React.Dispatch<React.SetStateAction<LineComment[]>>;
	setBranchName: (name: string | null) => void;

	selectWorktreeOptimistic: (path: string) => void;
	selectWorktree: (path: string) => void;
	resetPty: () => void;
}

export function useRemoteAppActions({
	send,
	disconnect,
	setConnection,
	activePtyId,
	setSelectedWorktree,
	setActiveTab,
	setTerminalMounted,
	setComments,
	setBranchName,
	selectWorktreeOptimistic,
	selectWorktree,
	resetPty,
}: UseRemoteAppActionsParams) {
	const handleSelectWorktree = useCallback(
		(worktreePath: string) => {
			selectWorktreeOptimistic(worktreePath);
			selectWorktree(worktreePath);
			setBranchName(null);
			resetPty();
			setActiveTab("terminal");
			setTerminalMounted(true);
		},
		[
			selectWorktreeOptimistic,
			selectWorktree,
			setBranchName,
			resetPty,
			setActiveTab,
			setTerminalMounted,
		],
	);

	const handleBackToWorktreesAction = useCallback(() => {
		setSelectedWorktree(null);
		setBranchName(null);
	}, [setSelectedWorktree, setBranchName]);

	const handleConnect = useCallback(
		(wsUrl: string, token: string) => {
			setConnection({ url: wsUrl, token });
		},
		[setConnection],
	);

	const handleDisconnect = useCallback(() => {
		disconnect();
		setConnection(null);
		resetPty();
	}, [disconnect, setConnection, resetPty]);

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent.map(lineCommentToThread));
			if (!text) return;
			if (activePtyId != null) {
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: text },
				});
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: "\r" },
				});
				setComments((prev) =>
					prev.map((c) =>
						unsent.some((u) => u.id === c.id)
							? { ...c, status: "sent" as const }
							: c,
					),
				);
			}
		},
		[send, activePtyId, setComments],
	);

	const handleSendComment = useCallback(
		(comment: LineComment) => {
			const text = formatCommentsForTerminal([lineCommentToThread(comment)]);
			if (!text) return;
			if (activePtyId != null) {
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: text },
				});
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: "\r" },
				});
				setComments((prev) =>
					prev.map((c) =>
						c.id === comment.id ? { ...c, status: "sent" as const } : c,
					),
				);
			}
		},
		[send, activePtyId, setComments],
	);

	const handleCopyComment = useCallback((comment: LineComment) => {
		const text = formatCommentForClipboard(lineCommentToThread(comment));
		navigator.clipboard.writeText(text).catch(() => {});
	}, []);

	const handleSendThreadsToTerminal = useCallback(
		(threadsToSend: Thread[]) => {
			const text = formatCommentsForTerminal(threadsToSend);
			if (!text) return;
			if (activePtyId != null) {
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: text },
				});
				send({
					type: "pty_input",
					payload: { pty_id: activePtyId, data: "\r" },
				});
			}
		},
		[send, activePtyId],
	);

	const handleCopyThread = useCallback((thread: Thread) => {
		const text = formatCommentForClipboard(thread);
		navigator.clipboard.writeText(text).catch(() => {});
	}, []);

	const handleTabChange = useCallback(
		(tab: Tab) => {
			setActiveTab(tab);
			if (tab === "terminal") setTerminalMounted(true);
		},
		[setActiveTab, setTerminalMounted],
	);

	return {
		handleSelectWorktree,
		handleBackToWorktreesAction,
		handleConnect,
		handleDisconnect,
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		handleSendThreadsToTerminal,
		handleCopyThread,
		handleTabChange,
	};
}
