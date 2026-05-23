import { History, X } from "lucide-react";
import type React from "react";
import { useCallback, useMemo, useRef, useState } from "react";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import type { MentionReference } from "@/types/session";
import { BoundSessionChat } from "./BoundSessionChat";

interface AgentChatPanelProps {
	worktreePath: string;
	registerDropZone: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
}

/**
 * spec issues-1023: 自由対話 chat の panel。タブバーは AgentChatPanel 固有の
 * drag&drop 並べ替え・history popover・新規作成ボタンを持つ。chat 本文と
 * MessageInput は {@link BoundSessionChat} に委譲され、WorkflowSidebarPanel と
 * 同一実装を共有する（issue #1023 「タブ含めて同じ UI で session フィルタだけが違う」設計）。
 */
export function AgentChatPanel({
	worktreePath,
	registerDropZone,
	sendMessageRef,
}: AgentChatPanelProps) {
	const {
		orderedSessions,
		closedSessions,
		activeSession,
		sessionAgentStates,
		selectSession,
		closeSession,
		restoreSession,
		createNewSession,
		reorderSessions,
		refreshClosedSessions,
	} = useAgentChatContext();

	// spec issues-1023: workflow step として起動された chat session は
	// 自由対話 chat tab と同格に tab bar 上に並べない。観測経路は Workflow panel の
	// step conversation transcript 側に切り出されている。
	const displayedSessions = useMemo(
		() => orderedSessions.filter((s) => !s.workflowStepSession),
		[orderedSessions],
	);
	const displayedClosedSessions = useMemo(
		() => closedSessions.filter((s) => !s.workflowStepSession),
		[closedSessions],
	);

	// spec issues-1023: 万一 activeSession が workflow step session の状態でも
	// AgentChatPanel 本文では表示しない（Workflow panel 側 transcript の二重表示防止）。
	const displayedActiveSession =
		activeSession && !activeSession.workflowStepSession ? activeSession : null;
	const activeSessionId = displayedActiveSession?.id ?? null;

	const [historyOpen, setHistoryOpen] = useState(false);
	const draggedSessionIdRef = useRef<string | null>(null);
	const isDraggingRef = useRef(false);

	const handleHistoryOpen = useCallback(
		(open: boolean) => {
			setHistoryOpen(open);
			if (open) {
				refreshClosedSessions();
			}
		},
		[refreshClosedSessions],
	);

	const handleRestore = useCallback(
		(sessionId: string) => {
			restoreSession(sessionId);
			setHistoryOpen(false);
		},
		[restoreSession],
	);

	const handleDragStart = useCallback(
		(e: React.DragEvent, sessionId: string) => {
			draggedSessionIdRef.current = sessionId;
			isDraggingRef.current = true;
			e.dataTransfer.effectAllowed = "move";
			e.dataTransfer.setData("text/plain", sessionId);
		},
		[],
	);

	const handleDragOver = useCallback((e: React.DragEvent) => {
		e.preventDefault();
		e.dataTransfer.dropEffect = "move";
	}, []);

	const handleDrop = useCallback(
		(e: React.DragEvent, targetId: string) => {
			e.preventDefault();
			const draggedId = draggedSessionIdRef.current;
			if (!draggedId || draggedId === targetId) return;

			// 並べ替えは tab bar 上に存在する free chat session 列に対してのみ意味がある。
			// workflow step session は tab bar に並ばないため、その index を参照しない。
			const currentOrder = displayedSessions.map((s) => s.id);
			const fromIndex = currentOrder.indexOf(draggedId);
			const toIndex = currentOrder.indexOf(targetId);
			if (fromIndex === -1 || toIndex === -1) return;

			const newOrder = [...currentOrder];
			newOrder.splice(fromIndex, 1);
			newOrder.splice(toIndex, 0, draggedId);

			// orderedSessions 全体の order に対しては、tab bar に並ばない session を
			// 既存位置に保ったまま、free chat session 部分だけを並べ替える。
			const hiddenIds = orderedSessions
				.map((s) => s.id)
				.filter((id) => !currentOrder.includes(id));
			reorderSessions([...newOrder, ...hiddenIds]);
		},
		[displayedSessions, orderedSessions, reorderSessions],
	);

	const handleDragEnd = useCallback(() => {
		draggedSessionIdRef.current = null;
		isDraggingRef.current = false;
	}, []);

	const handleTabClick = useCallback(
		(sessionId: string) => {
			if (isDraggingRef.current) return;
			selectSession(sessionId);
		},
		[selectSession],
	);

	return (
		<div data-testid="agent-chat-panel" className="flex flex-col h-full">
			<Tabs
				value={activeSessionId ?? ""}
				onValueChange={handleTabClick}
				className="flex flex-col h-full gap-0"
			>
				<div
					data-tauri-drag-region
					className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background border-b"
				>
					<TabsList
						data-testid="session-tab-list"
						className="w-auto max-w-full overflow-x-auto overflow-y-hidden justify-start [&::-webkit-scrollbar]:hidden [scrollbar-width:none]"
					>
						{displayedSessions.map((session) => (
							<TabsTrigger key={session.id} value={session.id} asChild>
								{/* biome-ignore lint/a11y/noStaticElementInteractions: TabsTrigger asChild が role を付与 */}
								{/* biome-ignore lint/a11y/useKeyWithClickEvents: TabsTrigger がキーボード操作を処理 */}
								<div
									className="gap-2"
									draggable={displayedSessions.length > 1}
									onDragStart={(e) => handleDragStart(e, session.id)}
									onDragOver={handleDragOver}
									onDrop={(e) => handleDrop(e, session.id)}
									onDragEnd={handleDragEnd}
									onClick={() => handleTabClick(session.id)}
								>
									<AgentStateIcon state={sessionAgentStates.get(session.id)} />
									<span className="truncate max-w-[120px]">
										{session.firstMessage || "New session"}
									</span>
									{displayedSessions.length > 1 && (
										<button
											type="button"
											onPointerDown={(e) => e.stopPropagation()}
											onMouseDown={(e) => e.stopPropagation()}
											onClick={(e) => {
												e.stopPropagation();
												closeSession(session.id);
											}}
											className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
											aria-label={`Close ${session.firstMessage || "New session"}`}
										>
											<X className="size-3.5" />
										</button>
									)}
								</div>
							</TabsTrigger>
						))}
					</TabsList>
					<div data-tauri-drag-region className="flex-1" />
					<button
						type="button"
						onClick={() => createNewSession()}
						aria-label="New session"
						className="px-2 h-full text-sm text-muted-foreground hover:text-foreground transition-colors shrink-0"
					>
						+
					</button>
					<Popover open={historyOpen} onOpenChange={handleHistoryOpen}>
						<PopoverTrigger asChild>
							<button
								type="button"
								aria-label="Session history"
								className="p-1 rounded hover:bg-muted-foreground/20 transition-colors shrink-0 ml-auto"
							>
								<History className="size-3.5" />
							</button>
						</PopoverTrigger>
						<PopoverContent align="end" className="w-64 p-0">
							{displayedClosedSessions.length > 0 ? (
								<ul className="max-h-60 overflow-y-auto">
									{displayedClosedSessions.map((session) => (
										<li key={session.id}>
											<button
												type="button"
												className="w-full text-left px-3 py-2 text-sm hover:bg-muted transition-colors truncate"
												onClick={() => handleRestore(session.id)}
											>
												{session.firstMessage || "New session"}
											</button>
										</li>
									))}
								</ul>
							) : (
								<p className="px-3 py-4 text-sm text-muted-foreground text-center">
									No closed sessions
								</p>
							)}
						</PopoverContent>
					</Popover>
				</div>
				{activeSessionId ? (
					<BoundSessionChat
						sessionId={activeSessionId}
						worktreePath={worktreePath}
						registerDropZone={registerDropZone}
						dropZoneName="agent"
						sendMessageRef={sendMessageRef}
						skipInitialLoad
					/>
				) : (
					<div className="flex-1 flex items-center justify-center text-sm text-muted-foreground">
						No chat selected
					</div>
				)}
			</Tabs>
		</div>
	);
}
