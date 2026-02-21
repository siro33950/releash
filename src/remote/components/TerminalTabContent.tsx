import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "../hooks/useMessageBus";
import type { ConnectionStatus } from "../hooks/useWebSocket";
import { RemoteTerminalPanel } from "./RemoteTerminalPanel";

interface PtySession {
	ptyId: number;
	cols: number;
}

interface TerminalTabContentProps {
	status: ConnectionStatus;
	ptySessions: PtySession[];
	activePtyId: number | null;
	ptySpawning: boolean;
	ptySpawnError: string | null;
	terminalMounted: boolean;
	selectedWorktree: string | null;
	activeTab: string;
	send: (msg: WsMessage) => void;
	subscribe: Subscribe;
	setActivePtyId: (id: number) => void;
	spawnPty: () => void;
}

export function TerminalTabContent({
	status,
	ptySessions,
	activePtyId,
	ptySpawning,
	ptySpawnError,
	terminalMounted,
	selectedWorktree,
	activeTab,
	send,
	subscribe,
	setActivePtyId,
	spawnPty,
}: TerminalTabContentProps) {
	if (terminalMounted && status === "connected" && ptySessions.length > 0) {
		return (
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
								ptySessions.find((s) => s.ptyId === activePtyId)?.cols ?? 80
							}
							send={send}
							subscribe={subscribe}
							visible={activeTab === "terminal"}
						/>
					</div>
				)}
			</>
		);
	}

	if (
		activeTab === "terminal" &&
		status === "connected" &&
		ptySessions.length === 0
	) {
		return (
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
		);
	}

	if (activeTab === "terminal" && status !== "connected") {
		return (
			<div className="flex items-center justify-center h-full text-muted-foreground">
				<p>接続されていません</p>
			</div>
		);
	}

	return null;
}
