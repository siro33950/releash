import { Bot, MessageSquare, Terminal } from "lucide-react";
import type { ComponentType } from "react";
import { useState } from "react";
import { ConnectionForm } from "./components/ConnectionForm";
import { RemoteAgentPanel } from "./components/RemoteAgentPanel";
import { RemoteAppHeader } from "./components/RemoteAppHeader";
import { RemoteDashboard } from "./components/RemoteDashboard";
import { RemoteReviewPanel } from "./components/RemoteReviewPanel";
import { TabBar } from "./components/TabBar";
import { TerminalTabContent } from "./components/TerminalTabContent";
import { useBrowserBackGuard } from "./hooks/useBrowserBackGuard";
import { useMessageBus } from "./hooks/useMessageBus";
import { usePtyManagement } from "./hooks/usePtyManagement";
import { useRemoteAppActions } from "./hooks/useRemoteAppActions";
import { useRemoteBackends } from "./hooks/useRemoteBackends";
import { useRemoteContent } from "./hooks/useRemoteContent";
import { type Tab, useRemoteNavigation } from "./hooks/useRemoteNavigation";
import { useRemoteReviewThreads } from "./hooks/useRemoteReviewThreads";
import { useRemoteWorkflowState } from "./hooks/useRemoteWorkflowState";
import { useRemoteWorktrees } from "./hooks/useRemoteWorktrees";
import { useWebSocket } from "./hooks/useWebSocket";

const tabs: {
	id: Tab;
	label: string;
	icon: ComponentType<{ className?: string }>;
}[] = [
	{ id: "terminal", label: "Terminal", icon: Terminal },
	{ id: "agent", label: "Agent", icon: Bot },
	{ id: "threads", label: "Threads", icon: MessageSquare },
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

	const { branchName, setBranchName } = useRemoteContent({ subscribe });

	const {
		backends,
		selectedBackendId,
		setSelectedBackendId,
		loading: backendLoading,
		refresh: refreshBackends,
	} = useRemoteBackends({
		subscribe,
		send,
		connected: status === "connected",
	});

	const { workflowState } = useRemoteWorkflowState({
		subscribe,
		selectedWorktree,
	});

	const reviewThreads = useRemoteReviewThreads({
		subscribe,
		send,
		connected: status === "connected",
		selectedWorktree,
	});

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
		handleTabChange,
	} = useRemoteAppActions({
		disconnect,
		setConnection,
		setSelectedWorktree,
		setActiveTab,
		setTerminalMounted,
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
				workflowState={workflowState}
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
						<div
							className="absolute inset-0 flex flex-col"
							style={{
								visibility: activeTab === "agent" ? "visible" : "hidden",
								pointerEvents: activeTab === "agent" ? "auto" : "none",
							}}
						>
							<RemoteAgentPanel
								selectedWorktree={selectedWorktree}
								backends={backends}
								selectedBackendId={selectedBackendId}
								backendLoading={backendLoading}
								status={status}
								send={send}
								subscribe={subscribe}
								onBackendChange={setSelectedBackendId}
								onRefreshBackends={refreshBackends}
							/>
						</div>
						<div
							className="absolute inset-0 flex flex-col"
							style={{
								visibility: activeTab === "threads" ? "visible" : "hidden",
								pointerEvents: activeTab === "threads" ? "auto" : "none",
							}}
						>
							<RemoteReviewPanel
								threads={reviewThreads.threads}
								selectedThread={reviewThreads.selectedThread}
								selectedThreadId={reviewThreads.selectedThreadId}
								loading={reviewThreads.loading}
								error={reviewThreads.error}
								onSelectThread={reviewThreads.setSelectedThreadId}
								onRefresh={reviewThreads.refresh}
								onCreateThread={reviewThreads.createThread}
								onAppendComment={reviewThreads.appendComment}
								onResolveThread={reviewThreads.resolveThread}
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
