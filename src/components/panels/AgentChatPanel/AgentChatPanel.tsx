import { invoke } from "@tauri-apps/api/core";
import { Archive, History, Search, X } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import {
	CommandDialog,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
	CommandShortcut,
} from "@/components/ui/command";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAgentChatContext } from "@/contexts/AgentChatContext";
import { useDisplayedActiveSession } from "@/hooks/useDisplayedActiveSession";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import type {
	AgentEditorSelection,
	MentionReference,
	SessionSearchResult,
} from "@/types/session";
import { BoundSessionChat } from "./BoundSessionChat";

interface AgentCommandPaletteItem {
	id: string;
	label: string;
	shortcut: string;
	alternateShortcut?: string | null;
	enabled: boolean;
}

interface AgentShortcutSetting {
	id: string;
	label: string;
	shortcut: string;
	alternateShortcut?: string | null;
	defaultShortcut: string;
}

function normalizeKeyboardShortcut(event: KeyboardEvent): string | null {
	const key = normalizeKeyboardKey(event.key);
	if (!key) return null;
	const parts: string[] = [];
	if (event.metaKey) parts.push("Cmd");
	if (event.ctrlKey) parts.push("Ctrl");
	if (event.altKey) parts.push("Alt");
	if (event.shiftKey) parts.push("Shift");
	if (parts.length === 0) return null;
	parts.push(key);
	return parts.join(" ");
}

function normalizeKeyboardKey(key: string): string | null {
	if (
		!key ||
		key === "Meta" ||
		key === "Control" ||
		key === "Alt" ||
		key === "Shift"
	) {
		return null;
	}
	if (key === " ") return "Space";
	if (key.length === 1) return key.toUpperCase();
	return key.slice(0, 1).toUpperCase() + key.slice(1).toLowerCase();
}

function shortcutMatches(
	setting: AgentShortcutSetting,
	shortcut: string | null,
): boolean {
	if (!shortcut) return false;
	return (
		setting.shortcut === shortcut || setting.alternateShortcut === shortcut
	);
}

function commandShortcutLabel(item: AgentCommandPaletteItem): string {
	if (item.alternateShortcut) {
		return `${item.shortcut} / ${item.alternateShortcut}`;
	}
	return item.shortcut;
}

interface AgentChatPanelProps {
	worktreePath: string;
	activeEditorPath?: string | null;
	openEditorPaths?: string[];
	activeEditorSelection?: AgentEditorSelection | null;
	registerDropZone: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
	onOpenDiffFile?: (filePath: string) => void;
}

/**
 * spec issues-1023: 自由対話 chat の panel。タブバーは AgentChatPanel 固有の
 * drag&drop 並べ替え・history popover・新規作成ボタンを持つ。chat 本文と
 * MessageInput は {@link BoundSessionChat} に委譲され、WorkflowView と
 * 同一実装を共有する（issue #1023 「タブ含めて同じ UI で session フィルタだけが違う」設計）。
 */
export function AgentChatPanel({
	worktreePath,
	activeEditorPath,
	openEditorPaths,
	activeEditorSelection,
	registerDropZone,
	sendMessageRef,
	onOpenDiffFile,
}: AgentChatPanelProps) {
	const {
		orderedSessions,
		closedSessions,
		sessionAgentStates,
		selectSession,
		closeSession,
		archiveSession,
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

	// spec issues-1023 / issues-1022: 万一 activeSession が workflow step session の状態でも
	// AgentChatPanel 本文では表示しない（Workflow panel 側 transcript の二重表示防止）。
	// Diff Thread handoff (issues-1022) と同じ判定規則を共通 hook 経由で参照する。
	const displayedActiveSession = useDisplayedActiveSession();
	const activeSessionId = displayedActiveSession?.id ?? null;

	const [historyOpen, setHistoryOpen] = useState(false);
	const [historySearchQuery, setHistorySearchQuery] = useState("");
	const [historySearchResults, setHistorySearchResults] = useState<
		SessionSearchResult[]
	>([]);
	const [historySearchError, setHistorySearchError] = useState<string | null>(
		null,
	);
	const [isHistorySearchLoading, setIsHistorySearchLoading] = useState(false);
	const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
	const [commandPaletteItems, setCommandPaletteItems] = useState<
		AgentCommandPaletteItem[]
	>([]);
	const [agentShortcuts, setAgentShortcuts] = useState<AgentShortcutSetting[]>(
		[],
	);
	const historySearchInputRef = useRef<HTMLInputElement>(null);
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

	const handleArchive = useCallback(
		(sessionId: string) => {
			archiveSession(sessionId);
		},
		[archiveSession],
	);

	useEffect(() => {
		let cancelled = false;
		void invoke<AgentShortcutSetting[]>("get_agent_shortcut_settings")
			.then((settings) => {
				if (cancelled) return;
				setAgentShortcuts(settings);
			})
			.catch(() => {
				if (cancelled) return;
				setAgentShortcuts([]);
			});
		return () => {
			cancelled = true;
		};
	}, []);

	useEffect(() => {
		if (!commandPaletteOpen) return;
		let cancelled = false;
		void invoke<AgentCommandPaletteItem[]>("present_agent_command_palette", {
			request: {
				hasActiveSession: Boolean(activeSessionId),
				sessionCount: displayedSessions.length,
			},
		})
			.then((items) => {
				if (cancelled) return;
				setCommandPaletteItems(items);
			})
			.catch(() => {
				if (cancelled) return;
				setCommandPaletteItems([]);
			});
		return () => {
			cancelled = true;
		};
	}, [activeSessionId, commandPaletteOpen, displayedSessions.length]);

	useEffect(() => {
		if (!historyOpen) return;
		const focusId = window.requestAnimationFrame(() => {
			historySearchInputRef.current?.focus();
		});
		return () => window.cancelAnimationFrame(focusId);
	}, [historyOpen]);

	useEffect(() => {
		if (!historyOpen) return;
		const query = historySearchQuery.trim();
		if (!query) {
			setHistorySearchResults([]);
			setHistorySearchError(null);
			setIsHistorySearchLoading(false);
			return;
		}
		let cancelled = false;
		setIsHistorySearchLoading(true);
		const timer = window.setTimeout(() => {
			invoke<SessionSearchResult[]>("search_agent_sessions", {
				worktreePath,
				query,
				includeWorkflow: false,
				limit: 20,
			})
				.then((results) => {
					if (cancelled) return;
					setHistorySearchResults(results);
					setHistorySearchError(null);
				})
				.catch((e) => {
					if (cancelled) return;
					setHistorySearchResults([]);
					setHistorySearchError(String(e));
				})
				.finally(() => {
					if (!cancelled) {
						setIsHistorySearchLoading(false);
					}
				});
		}, 150);
		return () => {
			cancelled = true;
			window.clearTimeout(timer);
		};
	}, [historyOpen, historySearchQuery, worktreePath]);

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
			// hidden を末尾に集約すると interleaved な配置（workflow step session が
			// free chat session の間に挟まる順序）が崩れるため、元の位置を保持する。
			let freeIndex = 0;
			const nextOrder = orderedSessions.map((s) =>
				currentOrder.includes(s.id) ? newOrder[freeIndex++] : s.id,
			);
			reorderSessions(nextOrder);
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

	const selectAdjacentSession = useCallback(
		(direction: -1 | 1) => {
			if (!activeSessionId || displayedSessions.length < 2) return;
			const currentIndex = displayedSessions.findIndex(
				(session) => session.id === activeSessionId,
			);
			if (currentIndex === -1) return;
			const nextIndex =
				(currentIndex + direction + displayedSessions.length) %
				displayedSessions.length;
			selectSession(displayedSessions[nextIndex].id);
		},
		[activeSessionId, displayedSessions, selectSession],
	);

	const runAgentCommand = useCallback(
		(commandId: string) => {
			setCommandPaletteOpen(false);
			switch (commandId) {
				case "command_menu":
					setCommandPaletteOpen(true);
					return;
				case "new_thread":
					createNewSession();
					return;
				case "search_threads":
					handleHistoryOpen(true);
					return;
				case "find_in_thread":
					window.dispatchEvent(new Event("agent-open-thread-find"));
					return;
				case "copy_latest_response":
					window.dispatchEvent(new Event("agent-copy-latest-response"));
					return;
				case "toggle_raw_scrollback":
					window.dispatchEvent(new Event("agent-toggle-raw-scrollback"));
					return;
				case "previous_thread":
					selectAdjacentSession(-1);
					return;
				case "next_thread":
					selectAdjacentSession(1);
					return;
			}
		},
		[createNewSession, handleHistoryOpen, selectAdjacentSession],
	);

	const runCommandPaletteItem = useCallback(
		(item: AgentCommandPaletteItem) => {
			if (!item.enabled) return;
			runAgentCommand(item.id);
		},
		[runAgentCommand],
	);

	useEffect(() => {
		const handleKeyDown = (event: KeyboardEvent) => {
			const shortcut = normalizeKeyboardShortcut(event);
			const matched = agentShortcuts.find((setting) =>
				shortcutMatches(setting, shortcut),
			);
			if (!matched) return;
			event.preventDefault();
			if (matched.id === "command_menu") {
				runAgentCommand(matched.id);
				return;
			}
			void invoke<boolean>("is_agent_command_enabled", {
				request: {
					commandId: matched.id,
					request: {
						hasActiveSession: Boolean(activeSessionId),
						sessionCount: displayedSessions.length,
					},
				},
			})
				.then((enabled) => {
					if (enabled !== true) return;
					runAgentCommand(matched.id);
				})
				.catch(() => {});
		};
		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [
		activeSessionId,
		agentShortcuts,
		displayedSessions.length,
		runAgentCommand,
	]);

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
								<div
									className="gap-2"
									draggable={displayedSessions.length > 1}
									onDragStart={(e) => handleDragStart(e, session.id)}
									onDragOver={handleDragOver}
									onDrop={(e) => handleDrop(e, session.id)}
									onDragEnd={handleDragEnd}
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
						<PopoverContent align="end" className="w-80 p-0">
							<div className="border-b p-2">
								<div className="flex items-center gap-1 rounded border px-2 py-1">
									<Search className="size-3.5 shrink-0 text-muted-foreground" />
									<input
										ref={historySearchInputRef}
										type="search"
										value={historySearchQuery}
										onChange={(event) =>
											setHistorySearchQuery(event.target.value)
										}
										placeholder="Search sessions"
										aria-label="Search sessions"
										className="min-w-0 flex-1 bg-transparent text-sm outline-none"
									/>
									{historySearchQuery && (
										<button
											type="button"
											aria-label="Clear session search"
											className="inline-flex size-5 shrink-0 items-center justify-center rounded hover:bg-muted"
											onClick={() => setHistorySearchQuery("")}
										>
											<X className="size-3" />
										</button>
									)}
								</div>
							</div>
							{historySearchQuery.trim() ? (
								<div className="max-h-72 overflow-y-auto">
									{isHistorySearchLoading ? (
										<p className="px-3 py-4 text-center text-sm text-muted-foreground">
											Searching...
										</p>
									) : historySearchError ? (
										<p className="px-3 py-4 text-center text-sm text-destructive">
											{historySearchError}
										</p>
									) : historySearchResults.length > 0 ? (
										<ul>
											{historySearchResults.map((result) => (
												<li key={result.session.id}>
													<div className="flex items-start gap-1 px-2 py-1 hover:bg-muted">
														<button
															type="button"
															className="min-w-0 flex-1 px-1 py-1 text-left"
															onClick={() => handleRestore(result.session.id)}
														>
															<div className="truncate text-sm">
																{result.session.firstMessage || "New session"}
															</div>
															<div className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
																{result.snippet}
															</div>
														</button>
														<button
															type="button"
															className="inline-flex size-7 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-background hover:text-foreground"
															aria-label={`Archive ${result.session.firstMessage || "New session"}`}
															title="Archive session"
															onClick={() => handleArchive(result.session.id)}
														>
															<Archive className="size-3.5" />
														</button>
													</div>
												</li>
											))}
										</ul>
									) : (
										<p className="px-3 py-4 text-center text-sm text-muted-foreground">
											No matching sessions
										</p>
									)}
								</div>
							) : displayedClosedSessions.length > 0 ? (
								<ul className="max-h-60 overflow-y-auto">
									{displayedClosedSessions.map((session) => (
										<li key={session.id}>
											<div className="flex items-center gap-1 px-2 py-1 hover:bg-muted">
												<button
													type="button"
													className="min-w-0 flex-1 truncate px-1 py-1 text-left text-sm"
													onClick={() => handleRestore(session.id)}
												>
													{session.firstMessage || "New session"}
												</button>
												<button
													type="button"
													className="inline-flex size-7 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-background hover:text-foreground"
													aria-label={`Archive ${session.firstMessage || "New session"}`}
													title="Archive session"
													onClick={() => handleArchive(session.id)}
												>
													<Archive className="size-3.5" />
												</button>
											</div>
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
						activeEditorPath={activeEditorPath}
						openEditorPaths={openEditorPaths}
						activeEditorSelection={activeEditorSelection}
						registerDropZone={registerDropZone}
						dropZoneName="agent"
						sendMessageRef={sendMessageRef}
						onOpenDiffFile={onOpenDiffFile}
						skipInitialLoad
					/>
				) : (
					<div className="flex-1 flex items-center justify-center text-sm text-muted-foreground">
						No chat selected
					</div>
				)}
				<CommandDialog
					open={commandPaletteOpen}
					onOpenChange={setCommandPaletteOpen}
					title="Agent Commands"
					description="Search agent commands"
					className="max-w-lg"
				>
					<CommandInput placeholder="Search commands" />
					<CommandList data-testid="agent-command-palette">
						<CommandEmpty>No commands found</CommandEmpty>
						<CommandGroup heading="Agent">
							{commandPaletteItems.map((item) => (
								<CommandItem
									key={item.id}
									value={item.label}
									disabled={!item.enabled}
									onSelect={() => runCommandPaletteItem(item)}
								>
									<span>{item.label}</span>
									<CommandShortcut>
										{commandShortcutLabel(item)}
									</CommandShortcut>
								</CommandItem>
							))}
						</CommandGroup>
					</CommandList>
				</CommandDialog>
			</Tabs>
		</div>
	);
}
