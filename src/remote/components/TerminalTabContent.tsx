import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "../hooks/useMessageBus";
import type { ConnectionStatus } from "../hooks/useWebSocket";
import { RemoteTerminalPanel } from "./RemoteTerminalPanel";

interface PtySession {
	ptyId: number;
	cols: number;
	label?: string;
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
	spawnPty: (label?: string) => void;
	killPty: (ptyId: number) => void;
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
	killPty,
}: TerminalTabContentProps) {
	if (terminalMounted && status === "connected" && ptySessions.length > 0) {
		return (
			<>
				<div className="flex items-center gap-1 px-2 py-1 border-b border-border bg-card shrink-0 overflow-x-auto">
					{ptySessions.map((s, index) => (
						<div
							key={s.ptyId}
							className={`group flex items-center gap-1 px-2 py-0.5 text-xs rounded transition-colors shrink-0 cursor-pointer ${
								activePtyId === s.ptyId
									? "bg-primary text-primary-foreground"
									: "bg-secondary text-muted-foreground hover:bg-secondary/80"
							}`}
							role="tab"
							tabIndex={0}
							aria-selected={activePtyId === s.ptyId}
							onClick={() => setActivePtyId(s.ptyId)}
							onKeyDown={(e) => {
								if (e.key === "Enter") setActivePtyId(s.ptyId);
							}}
						>
							<span>{s.label ?? `Terminal ${index + 1}`}</span>
							<button
								type="button"
								className={`ml-0.5 rounded-sm hover:bg-black/20 inline-flex items-center ${
									activePtyId === s.ptyId
										? "opacity-80"
										: "opacity-0 group-hover:opacity-60"
								}`}
								onClick={(e) => {
									e.stopPropagation();
									killPty(s.ptyId);
								}}
								aria-label={`Close ${s.label ?? `Terminal ${index + 1}`}`}
							>
								&#x2715;
							</button>
						</div>
					))}
					<button
						type="button"
						className="px-1.5 py-0.5 text-xs text-muted-foreground hover:text-foreground hover:bg-secondary rounded transition-colors shrink-0"
						onClick={() => spawnPty()}
						disabled={ptySpawning || !selectedWorktree}
						aria-label="Add terminal"
					>
						+
					</button>
				</div>
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
					onClick={() => spawnPty()}
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
