import { MessageSquare, MessageSquareDashed, Terminal } from "lucide-react";
import { useState } from "react";
import { ConnectionForm } from "./components/ConnectionForm";
import { RemoteAppHeader } from "./components/RemoteAppHeader";
import { RemoteCommentList } from "./components/RemoteCommentList";
import { RemoteDashboard } from "./components/RemoteDashboard";
import { RemoteThreadList } from "./components/RemoteThreadList";
import { TabBar } from "./components/TabBar";
import { TerminalTabContent } from "./components/TerminalTabContent";
import { useBrowserBackGuard } from "./hooks/useBrowserBackGuard";
import { useMessageBus } from "./hooks/useMessageBus";
import { usePtyManagement } from "./hooks/usePtyManagement";
import { useRemoteAppActions } from "./hooks/useRemoteAppActions";
import { useRemoteContent } from "./hooks/useRemoteContent";
import { type Tab, useRemoteNavigation } from "./hooks/useRemoteNavigation";
import { useRemoteThreads } from "./hooks/useRemoteThreads";
import { useRemoteWorktrees } from "./hooks/useRemoteWorktrees";
import { useWebSocket } from "./hooks/useWebSocket";

const tabs: { id: Tab; label: string; icon: typeof Terminal }[] = [
	{ id: "terminal", label: "Terminal", icon: Terminal },
	{ id: "comments", label: "Comments", icon: MessageSquare },
	{ id: "threads", label: "Threads", icon: MessageSquareDashed },
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
		selectedWorktree,
		worktreeLoading,
		activeTab,
		setSelectedWorktree,
		setActiveTab,
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
		killPty,
		resetPty,
	} = usePtyManagement({ subscribe, send });

	const {
		comments,
		branchName,
		setComments,
		setBranchName,
		deleteComment,
		updateComment,
	} = useRemoteContent({ subscribe, send });

	const {
		threads,
		addEntry: addThreadEntry,
		resolveThread,
		deleteThread,
	} = useRemoteThreads({ subscribe, send });
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
		handleSendToTerminal,
		handleSendComment,
		handleCopyComment,
		handleSendThreadsToTerminal,
		handleCopyThread,
		handleTabChange,
	} = useRemoteAppActions({
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
							className="absolute inset-0"
							style={{ display: activeTab === "threads" ? undefined : "none" }}
						>
							<RemoteThreadList
								threads={threads}
								onSendToTerminal={handleSendThreadsToTerminal}
								onDeleteThread={deleteThread}
								onResolveThread={resolveThread}
								onAddEntry={addThreadEntry}
								onCopyThread={handleCopyThread}
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
								killPty={killPty}
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
