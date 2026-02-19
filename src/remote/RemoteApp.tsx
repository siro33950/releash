import {
	ArrowLeft,
	FileDiff,
	GitBranch,
	MessageSquare,
	Terminal,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { computeHunks } from "@/lib/computeHunks";
import { formatCommentForClipboard } from "@/lib/formatCommentForClipboard";
import { formatCommentsForTerminal } from "@/lib/formatCommentsForTerminal";
import { generatePatch } from "@/lib/generatePatch";
import type { LineComment } from "@/types/comment";
import { ConnectionForm } from "./components/ConnectionForm";
import { RemoteCommentList } from "./components/RemoteCommentList";
import { RemoteDashboard } from "./components/RemoteDashboard";
import { RemoteDiffPanel } from "./components/RemoteDiffPanel";
import { RemoteSourceControl } from "./components/RemoteSourceControl";
import { RemoteTerminalPanel } from "./components/RemoteTerminalPanel";
import { StatusIndicator } from "./components/StatusIndicator";
import { useAgentState } from "./hooks/useAgentState";
import { useBrowserBackGuard } from "./hooks/useBrowserBackGuard";
import { useMessageBus } from "./hooks/useMessageBus";
import { usePtyManagement } from "./hooks/usePtyManagement";
import { useRemoteContent } from "./hooks/useRemoteContent";
import {
	type DiffBase,
	useRemoteFileContent,
} from "./hooks/useRemoteFileContent";
import { useRemoteGitActions } from "./hooks/useRemoteGitActions";
import { useRemoteGitStatus } from "./hooks/useRemoteGitStatus";
import { type Tab, useRemoteNavigation } from "./hooks/useRemoteNavigation";
import { useRemoteWorktrees } from "./hooks/useRemoteWorktrees";
import { useWebSocket } from "./hooks/useWebSocket";

const tabs: { id: Tab; label: string; icon: typeof GitBranch }[] = [
	{ id: "terminal", label: "Terminal", icon: Terminal },
	{ id: "changes", label: "Changes", icon: GitBranch },
	{ id: "diff", label: "Diff", icon: FileDiff },
	{ id: "comments", label: "Comments", icon: MessageSquare },
];

export function RemoteApp() {
	const [connection, setConnection] = useState<{
		url: string;
		token: string;
	} | null>(null);

	const { dispatch, subscribe } = useMessageBus();

	const { status, send, disconnect } = useWebSocket({
		url: connection?.url ?? "",
		token: connection?.token ?? "",
		onMessage: dispatch,
	});

	const {
		selectedPath,
		selectedWorktree,
		worktreeLoading,
		activeTab,
		diffBase,
		setSelectedPath,
		setSelectedWorktree,
		setActiveTab,
		setDiffBase,
		selectWorktreeOptimistic,
	} = useRemoteNavigation({ subscribe });

	const {
		ptySessions,
		activePtyId,
		ptySpawning,
		ptySpawnError,
		terminalMounted,
		setActivePtyId,
		setTerminalMounted,
		spawnPty,
		resetPty,
	} = usePtyManagement({ subscribe, send });

	const {
		comments,
		branchName,
		setComments,
		setBranchName,
		addComment,
		deleteComment,
		updateComment,
	} = useRemoteContent({ subscribe, send });

	const { stagedFiles, changedFiles } = useRemoteGitStatus({ subscribe });
	const { content, loading, requestContent } = useRemoteFileContent({
		subscribe,
		send,
	});
	const {
		stage,
		unstage,
		stageHunk,
		commit,
		push,
		committing,
		pushing,
		pushResult,
		clearPushResult,
		error,
		clearError,
	} = useRemoteGitActions({
		send,
		subscribe,
	});
	const { agentStates } = useAgentState({ subscribe });

	const {
		worktrees,
		loading: worktreesLoading,
		refresh: refreshWorktrees,
		select: selectWorktree,
	} = useRemoteWorktrees({
		subscribe,
		send,
		connected: status === "connected",
	});

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

	const { navigateBack: handleBackToWorktrees } = useBrowserBackGuard({
		selectedWorktree,
		onBack: handleBackToWorktreesAction,
	});

	const handleConnect = useCallback((wsUrl: string, token: string) => {
		setConnection({ url: wsUrl, token });
	}, []);

	const handleDisconnect = useCallback(() => {
		disconnect();
		setConnection(null);
		resetPty();
	}, [disconnect, resetPty]);

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

	if (!connection) {
		return <ConnectionForm onConnect={handleConnect} />;
	}

	return (
		<div className="flex flex-col h-dvh bg-background text-foreground">
			<header className="flex items-center justify-between px-3 py-1.5 border-b border-border bg-card shrink-0">
				<div className="flex items-center gap-2 min-w-0">
					{selectedWorktree && (
						<button
							type="button"
							onClick={handleBackToWorktrees}
							className="p-1 -ml-1 rounded hover:bg-muted transition-colors shrink-0"
							aria-label="Back"
						>
							<ArrowLeft className="size-4" />
						</button>
					)}
					<h1 className="text-sm font-semibold shrink-0">Releash Remote</h1>
					{branchName && (
						<span className="text-xs text-muted-foreground truncate font-mono">
							{branchName}
						</span>
					)}
				</div>
				<div className="flex items-center gap-2 shrink-0">
					<StatusIndicator status={status} />
					<button
						type="button"
						onClick={handleDisconnect}
						className="text-xs px-2 py-0.5 rounded bg-secondary hover:bg-secondary/80 text-secondary-foreground transition-colors"
					>
						切断
					</button>
				</div>
			</header>

			{selectedWorktree === null ? (
				<main className="flex-1 overflow-hidden">
					<RemoteDashboard
						worktrees={worktrees}
						loading={worktreesLoading}
						onRefresh={refreshWorktrees}
						onSelect={handleSelectWorktree}
						agentStates={agentStates}
					/>
				</main>
			) : (
				<>
					<main className="flex-1 overflow-hidden relative">
						{worktreeLoading && (
							<div className="absolute inset-0 flex items-center justify-center bg-background/80 z-10">
								<div className="animate-spin size-6 border-2 border-muted-foreground border-t-primary rounded-full" />
							</div>
						)}
						<div
							className="absolute inset-0"
							style={{ display: activeTab === "changes" ? undefined : "none" }}
						>
							<RemoteSourceControl
								stagedFiles={stagedFiles}
								changedFiles={changedFiles}
								selectedPath={selectedPath}
								onSelectFile={handleSelectFile}
								onStage={stage}
								onUnstage={unstage}
								onCommit={commit}
								onPush={push}
								committing={committing}
								pushing={pushing}
								pushResult={pushResult}
								onClearPushResult={clearPushResult}
								error={error}
								onClearError={clearError}
								onNavigateToDiff={handleNavigateToDiff}
								onRefresh={handleRefreshStatus}
							/>
						</div>

						<div
							className="absolute inset-0 flex flex-col"
							style={{ display: activeTab === "diff" ? undefined : "none" }}
						>
							{selectedPath && (
								<div className="flex items-center justify-between gap-2 px-3 py-1 border-b border-border bg-card shrink-0">
									<span className="text-xs text-muted-foreground truncate flex-1 min-w-0">
										{selectedPath}
									</span>
									<div className="flex items-center gap-1.5 shrink-0">
										<select
											value={diffBase}
											onChange={(e) =>
												handleDiffBaseChange(e.target.value as DiffBase)
											}
											className="text-xs bg-input text-secondary-foreground border border-border rounded px-1.5 py-0.5"
										>
											<option value="HEAD">HEAD</option>
											<option value="staged">Staged</option>
										</select>
										{hasDiffChanges && (
											<button
												type="button"
												onClick={handleStageAll}
												className="text-xs px-2 py-0.5 rounded bg-success/80 hover:bg-success/70 text-success-foreground transition-colors"
											>
												Stage All
											</button>
										)}
										{diffBase === "HEAD" &&
											stagedFiles.some((f) => f.path === selectedPath) && (
												<button
													type="button"
													onClick={handleUnstageAll}
													className="text-xs px-2 py-0.5 rounded bg-warning/80 hover:bg-warning/70 text-warning-foreground transition-colors"
												>
													Unstage All
												</button>
											)}
									</div>
								</div>
							)}
							<div className="flex-1" style={{ minHeight: 0 }}>
								{status === "connected" ? (
									<RemoteDiffPanel
										path={selectedPath}
										original={content?.original ?? ""}
										modified={content?.modified ?? ""}
										loading={loading}
										diffBase={diffBase}
										staged={content?.staged ?? null}
										onStageHunk={stageHunk}
										onAddComment={addComment}
									/>
								) : (
									<div className="flex items-center justify-center h-full text-muted-foreground">
										<p>接続中...</p>
									</div>
								)}
							</div>
						</div>

						<div
							className="absolute inset-0"
							style={{ display: activeTab === "comments" ? undefined : "none" }}
						>
							<RemoteCommentList
								comments={comments}
								onSendToTerminal={handleSendToTerminal}
								onDeleteComment={deleteComment}
								onUpdateComment={updateComment}
								onSendComment={handleSendComment}
								onCopyComment={handleCopyComment}
							/>
						</div>

						<div
							className="absolute inset-0 flex flex-col"
							style={{
								visibility: activeTab === "terminal" ? "visible" : "hidden",
								pointerEvents: activeTab === "terminal" ? "auto" : "none",
							}}
						>
							{terminalMounted &&
							status === "connected" &&
							ptySessions.length > 0 ? (
								<>
									{ptySessions.length > 1 && (
										<div className="flex items-center gap-1 px-2 py-1 border-b border-border bg-card shrink-0 overflow-x-auto">
											{ptySessions.map((s) => (
												<button
													key={s.ptyId}
													type="button"
													className={`px-2 py-0.5 text-xs rounded transition-colors shrink-0 ${
														activePtyId === s.ptyId
															? "bg-primary text-primary-foreground"
															: "bg-secondary text-muted-foreground hover:bg-secondary/80"
													}`}
													onClick={() => setActivePtyId(s.ptyId)}
												>
													PTY {s.ptyId}
												</button>
											))}
										</div>
									)}
									{activePtyId != null && (
										<div className="flex-1" style={{ minHeight: 0 }}>
											<RemoteTerminalPanel
												key={activePtyId}
												ptyId={activePtyId}
												ptyCols={
													ptySessions.find((s) => s.ptyId === activePtyId)
														?.cols ?? 80
												}
												send={send}
												subscribe={subscribe}
												visible={activeTab === "terminal"}
											/>
										</div>
									)}
								</>
							) : activeTab === "terminal" &&
								status === "connected" &&
								ptySessions.length === 0 ? (
								<div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground">
									<p>ターミナルセッションがありません</p>
									<button
										type="button"
										onClick={spawnPty}
										disabled={ptySpawning || !selectedWorktree}
										className="px-4 py-2 rounded bg-primary hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed text-primary-foreground text-sm transition-colors"
									>
										{ptySpawning ? "起動中..." : "ターミナルを起動"}
									</button>
									{ptySpawnError && (
										<p className="text-destructive text-xs">{ptySpawnError}</p>
									)}
									{!selectedWorktree && (
										<p className="text-muted-foreground text-xs">
											Worktreeを選択してください
										</p>
									)}
								</div>
							) : activeTab === "terminal" && status !== "connected" ? (
								<div className="flex items-center justify-center h-full text-muted-foreground">
									<p>接続されていません</p>
								</div>
							) : null}
						</div>
					</main>

					<nav className="flex shrink-0 border-t border-border bg-card">
						{tabs.map((tab) => {
							const Icon = tab.icon;
							const isActive = activeTab === tab.id;
							return (
								<button
									key={tab.id}
									type="button"
									className={`flex-1 flex flex-col items-center justify-center h-12 gap-0.5 transition-colors ${
										isActive
											? "text-primary border-t-2 border-primary"
											: "text-muted-foreground"
									}`}
									onClick={() => {
										setActiveTab(tab.id);
										if (tab.id === "terminal") setTerminalMounted(true);
									}}
								>
									<Icon className="h-4 w-4" />
									<span className="text-[10px]">{tab.label}</span>
								</button>
							);
						})}
					</nav>
				</>
			)}
		</div>
	);
}
