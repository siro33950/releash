import { FileDiff, GitBranch, MessageSquare, Terminal } from "lucide-react";
import { useState } from "react";
import { ConnectionForm } from "./components/ConnectionForm";
import { DiffTabContent } from "./components/DiffTabContent";
import { RemoteAppHeader } from "./components/RemoteAppHeader";
import { RemoteCommentList } from "./components/RemoteCommentList";
import { RemoteDashboard } from "./components/RemoteDashboard";
import { RemoteSourceControl } from "./components/RemoteSourceControl";
import { TabBar } from "./components/TabBar";
import { TerminalTabContent } from "./components/TerminalTabContent";
import { useAgentState } from "./hooks/useAgentState";
import { useBrowserBackGuard } from "./hooks/useBrowserBackGuard";
import { useMessageBus } from "./hooks/useMessageBus";
import { usePtyManagement } from "./hooks/usePtyManagement";
import { useRemoteAppActions } from "./hooks/useRemoteAppActions";
import { useRemoteContent } from "./hooks/useRemoteContent";
import { useRemoteFileContent } from "./hooks/useRemoteFileContent";
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

	const {
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
	} = useRemoteAppActions({
		send,
		disconnect,
		setConnection,
		selectedPath,
		selectedWorktree,
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
	});

	const { navigateBack: handleBackToWorktrees } = useBrowserBackGuard({
		selectedWorktree,
		onBack: handleBackToWorktreesAction,
	});

	if (!connection) {
		return <ConnectionForm onConnect={handleConnect} />;
	}

	return (
		<div className="flex flex-col h-dvh bg-background text-foreground">
			<RemoteAppHeader
				selectedWorktree={selectedWorktree}
				branchName={branchName}
				status={status}
				onBackToWorktrees={handleBackToWorktrees}
				onDisconnect={handleDisconnect}
			/>

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
							<DiffTabContent
								status={status}
								selectedPath={selectedPath}
								diffBase={diffBase}
								hasDiffChanges={hasDiffChanges}
								stagedFiles={stagedFiles}
								content={content}
								loading={loading}
								onDiffBaseChange={handleDiffBaseChange}
								onStageAll={handleStageAll}
								onUnstageAll={handleUnstageAll}
								onStageHunk={stageHunk}
								onAddComment={addComment}
							/>
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
							<TerminalTabContent
								status={status}
								ptySessions={ptySessions}
								activePtyId={activePtyId}
								ptySpawning={ptySpawning}
								ptySpawnError={ptySpawnError}
								terminalMounted={terminalMounted}
								selectedWorktree={selectedWorktree}
								activeTab={activeTab}
								send={send}
								subscribe={subscribe}
								setActivePtyId={setActivePtyId}
								spawnPty={spawnPty}
							/>
						</div>
					</main>

					<TabBar
						tabs={tabs}
						activeTab={activeTab}
						onTabChange={handleTabChange}
					/>
				</>
			)}
		</div>
	);
}
