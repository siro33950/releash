import { useCallback, useMemo } from "react";
import { computeHunks } from "@/lib/computeHunks";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { generatePatch } from "@/lib/generatePatch";
import type { LineComment } from "@/types/comment";
import type { WsMessage } from "@/types/protocol";
import type { DiffBase } from "./useRemoteFileContent";
import type { Tab } from "./useRemoteNavigation";

interface FileContent {
	path: string;
	original: string;
	modified: string;
	staged: string | null;
}

interface UseRemoteAppActionsParams {
	send: (msg: WsMessage) => void;
	disconnect: () => void;
	setConnection: (value: { url: string; token: string } | null) => void;

	selectedPath: string | null;
	selectedWorktree: string | null;
	diffBase: DiffBase;
	content: FileContent | null;
	activePtyId: number | null;

	setSelectedPath: (path: string | null) => void;
	setSelectedWorktree: (worktree: string | null) => void;
	setActiveTab: (tab: Tab) => void;
	setDiffBase: (base: DiffBase) => void;
	setTerminalMounted: (mounted: boolean) => void;
	setComments: React.Dispatch<React.SetStateAction<LineComment[]>>;
	setBranchName: (name: string | null) => void;

	selectWorktreeOptimistic: (path: string) => void;
	selectWorktree: (path: string) => void;
	resetPty: () => void;
	requestContent: (path: string, diffBase?: DiffBase) => void;
	stageHunk: (patch: string) => void;
}

export function useRemoteAppActions({
	send,
	disconnect,
	setConnection,
	selectedPath,
	diffBase,
	content,
	activePtyId,
	setSelectedPath,
	setSelectedWorktree,
	setActiveTab,
	setDiffBase,
	setTerminalMounted,
	setComments,
	setBranchName,
	selectWorktreeOptimistic,
	selectWorktree,
	resetPty,
	requestContent,
	stageHunk,
}: UseRemoteAppActionsParams) {
	const handleSelectWorktree = useCallback(
		(worktreePath: string) => {
			selectWorktreeOptimistic(worktreePath);
			selectWorktree(worktreePath);
			setSelectedPath(null);
			setBranchName(null);
			resetPty();
			setActiveTab("terminal");
			setTerminalMounted(true);
		},
		[
			selectWorktreeOptimistic,
			selectWorktree,
			setSelectedPath,
			setBranchName,
			resetPty,
			setActiveTab,
			setTerminalMounted,
		],
	);

	const handleBackToWorktreesAction = useCallback(() => {
		setSelectedWorktree(null);
		setSelectedPath(null);
		setBranchName(null);
	}, [setSelectedWorktree, setSelectedPath, setBranchName]);

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

	const handleSelectFile = useCallback(
		(path: string) => {
			setSelectedPath(path);
			requestContent(path, diffBase);
		},
		[setSelectedPath, requestContent, diffBase],
	);

	const handleDiffBaseChange = useCallback(
		(newBase: DiffBase) => {
			setDiffBase(newBase);
			if (selectedPath) {
				requestContent(selectedPath, newBase);
			}
		},
		[selectedPath, setDiffBase, requestContent],
	);

	const handleNavigateToDiff = useCallback(() => {
		setActiveTab("diff");
	}, [setActiveTab]);

	const handleRefreshStatus = useCallback(() => {
		send({ type: "git_status_request", payload: {} as Record<string, never> });
	}, [send]);

	const handleSendToTerminal = useCallback(
		(unsent: LineComment[]) => {
			const text = formatCommentsForTerminal(unsent);
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
			const text = formatCommentsForTerminal([comment]);
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
		const text = formatCommentForClipboard(comment);
		navigator.clipboard.writeText(text).catch(() => {});
	}, []);

	const hasDiffChanges = useMemo(() => {
		if (!content) return false;
		return content.original !== content.modified;
	}, [content]);

	const handleStageAll = useCallback(() => {
		if (!selectedPath || !content) return;
		const base =
			diffBase === "HEAD" && content.staged != null
				? content.staged
				: content.original;
		const allHunks = computeHunks(base, content.modified, selectedPath);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(selectedPath, allHunks, allIndices);
		if (patch) stageHunk(patch);
	}, [selectedPath, content, diffBase, stageHunk]);

	const handleUnstageAll = useCallback(() => {
		if (!selectedPath || !content || content.staged == null) return;
		const allHunks = computeHunks(
			content.staged,
			content.original,
			selectedPath,
		);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(selectedPath, allHunks, allIndices);
		if (patch) stageHunk(patch);
	}, [selectedPath, content, stageHunk]);

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
		handleSelectFile,
		handleDiffBaseChange,
		handleNavigateToDiff,
		handleRefreshStatus,
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		hasDiffChanges,
		handleStageAll,
		handleUnstageAll,
		handleTabChange,
	};
}
