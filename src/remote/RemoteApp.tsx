import { Terminal } from "lucide-react";
import { useState } from "react";
import { ConnectionForm } from "./components/ConnectionForm";
import { RemoteAppHeader } from "./components/RemoteAppHeader";
import { RemoteDashboard } from "./components/RemoteDashboard";
import { TabBar } from "./components/TabBar";
import { TerminalTabContent } from "./components/TerminalTabContent";
import { useBrowserBackGuard } from "./hooks/useBrowserBackGuard";
import { useMessageBus } from "./hooks/useMessageBus";
import { usePtyManagement } from "./hooks/usePtyManagement";
import { useRemoteAppActions } from "./hooks/useRemoteAppActions";
import { useRemoteContent } from "./hooks/useRemoteContent";
import { type Tab, useRemoteNavigation } from "./hooks/useRemoteNavigation";
import { useRemoteWorkflowState } from "./hooks/useRemoteWorkflowState";
import { useRemoteWorktrees } from "./hooks/useRemoteWorktrees";
import { useWebSocket } from "./hooks/useWebSocket";

const tabs: { id: Tab; label: string; icon: typeof Terminal }[] = [
	{ id: "terminal", label: "Terminal", icon: Terminal },
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

	const { workflowState } = useRemoteWorkflowState({
		subscribe,
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
