import { History, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAgentChat } from "@/hooks/useAgentChat";
import { ActivityItem, CollapsibleError, ToolActivity } from "./ActivityLog";
import { MessageInput } from "./MessageInput";
import { MODES } from "./ModeSelector";
import { PermissionDialog } from "./PermissionDialog";
import { ShimmerPlaceholder } from "./ShimmerPlaceholder";
import { StreamMessage } from "./StreamMessage";

interface AgentChatPanelProps {
	worktreePath: string;
}

export function AgentChatPanel({ worktreePath }: AgentChatPanelProps) {
	const {
		sessions,
		orderedSessions,
		closedSessions,
		activeSession,
		isStreaming,
		error,
		permissionMode,
		pendingPermission,
		sendMessage,
		interrupt,
		selectSession,
		closeSession,
		restoreSession,
		createNewSession,
		reorderSessions,
		setPermissionMode,
		respondPermission,
		refreshClosedSessions,
	} = useAgentChat(worktreePath);

	const isWaiting = isStreaming && pendingPermission !== null;
	const [historyOpen, setHistoryOpen] = useState(false);
	const draggedSessionIdRef = useRef<string | null>(null);
	const isDraggingRef = useRef(false);

	const scrollRef = useRef<HTMLDivElement>(null);
	const lastMessageCount = useRef(0);

	// Auto-scroll to bottom when messages are added
	useEffect(() => {
		const el = scrollRef.current;
		if (!el) return;
		const count = activeSession?.messages.length ?? 0;
		if (count > lastMessageCount.current) {
			el.scrollTop = el.scrollHeight;
		}
		lastMessageCount.current = count;
	}, [activeSession?.messages.length]);

	// Also scroll when streaming content updates
	const agentMessages = activeSession?.messages.filter(
		(m) => m.role === "agent",
	);
	const lastAgentMsg = agentMessages?.[agentMessages.length - 1];
	const lastAgentPartsLen = lastAgentMsg?.parts.length ?? 0;
	const lastAgentContent =
		lastAgentMsg?.parts
			.filter((p) => p.type === "text")
			.reduce((len, p) => len + (p as { content: string }).content.length, 0) ??
		0;

	// biome-ignore lint/correctness/useExhaustiveDependencies: lastAgentContent/lastAgentPartsLen triggers scroll on content growth
	useEffect(() => {
		if (!isStreaming) return;
		const el = scrollRef.current;
		if (!el) return;
		const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
		if (isNearBottom) {
			el.scrollTop = el.scrollHeight;
		}
	}, [isStreaming, lastAgentContent, lastAgentPartsLen]);

	const msgs = activeSession?.messages;
	const lastMsg = msgs?.[msgs.length - 1];
	const showWaitingShimmer =
		isStreaming && lastMsg?.role === "agent" && lastMsg.parts.length === 0;

	const isInputDisabled = isStreaming;

	const cycleMode = useCallback(() => {
		const currentIndex = MODES.findIndex((m) => m.value === permissionMode);
		const nextIndex = (currentIndex + 1) % MODES.length;
		setPermissionMode(MODES[nextIndex].value);
	}, [permissionMode, setPermissionMode]);

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

			const currentOrder = orderedSessions.map((s) => s.id);
			const fromIndex = currentOrder.indexOf(draggedId);
			const toIndex = currentOrder.indexOf(targetId);
			if (fromIndex === -1 || toIndex === -1) return;

			const newOrder = [...currentOrder];
			newOrder.splice(fromIndex, 1);
			newOrder.splice(toIndex, 0, draggedId);
			reorderSessions(newOrder);
		},
		[orderedSessions, reorderSessions],
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
				value={activeSession?.id ?? ""}
				onValueChange={selectSession}
				className="flex flex-col h-full gap-0"
			>
				<div className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background border-b">
					<TabsList
						data-testid="session-tab-list"
						className="w-auto max-w-full overflow-x-auto overflow-y-hidden justify-start [&::-webkit-scrollbar]:hidden [scrollbar-width:none]"
					>
						{orderedSessions.map((session) => (
							<TabsTrigger key={session.id} value={session.id} asChild>
								{/* biome-ignore lint/a11y/noStaticElementInteractions: TabsTrigger asChild が role を付与 */}
								{/* biome-ignore lint/a11y/useKeyWithClickEvents: TabsTrigger がキーボード操作を処理 */}
								<div
									className="gap-2"
									draggable={sessions.length > 1}
									onDragStart={(e) => handleDragStart(e, session.id)}
									onDragOver={handleDragOver}
									onDrop={(e) => handleDrop(e, session.id)}
									onDragEnd={handleDragEnd}
									onClick={() => handleTabClick(session.id)}
								>
									<span className="truncate max-w-[120px]">
										{session.firstMessage || "New session"}
									</span>
									{sessions.length > 1 && (
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
							{closedSessions.length > 0 ? (
								<ul className="max-h-60 overflow-y-auto">
									{closedSessions.map((session) => (
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
				<div className="flex flex-col flex-1 min-h-0">
					{isStreaming && (
						<div
							data-testid="agent-state-indicator"
							className="px-4 py-1 bg-muted text-muted-foreground text-xs border-b"
						>
							{isWaiting ? "Waiting..." : "Running..."}
						</div>
					)}
					<div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto">
						{activeSession && (
							<div className="py-2">
								{activeSession.messages.map((msg, idx) => {
									if (msg.role !== "agent") {
										const textContent = msg.parts
											.filter((p) => p.type === "text")
											.map((p) => (p as { content: string }).content)
											.join("");
										return (
											<div key={msg.id}>
												<StreamMessage
													content={textContent}
													role={msg.role}
													isStreaming={false}
												/>
											</div>
										);
									}

									const isLastMsg = idx === activeSession.messages.length - 1;
									const isLastAgentStreaming = isStreaming && isLastMsg;

									return (
										<div key={msg.id}>
											{/* biome-ignore lint/suspicious/useIterableCallbackReturn: switch is exhaustive for MessagePart */}
											{msg.parts.map((part, i) => {
												const key = `${msg.id}-p${i}`;
												const nextPart = msg.parts[i + 1];
												const isLastPart = i === msg.parts.length - 1;
												const partStreaming =
													isLastAgentStreaming && isLastPart;

												// Skip tool_result that is paired with preceding tool_use
												if (
													part.type === "tool_result" &&
													i > 0 &&
													msg.parts[i - 1].type === "tool_use"
												)
													return null;

												switch (part.type) {
													case "thinking":
														if (isLastAgentStreaming)
															return <ShimmerPlaceholder key={key} lines={2} />;
														return null;
													case "text":
														return (
															// biome-ignore lint/a11y/useValidAriaRole: role is a component prop, not an ARIA role
															<StreamMessage
																key={key}
																content={part.content}
																role="agent"
																isStreaming={partStreaming}
															/>
														);
													case "error":
														return (
															<div key={key} className="px-5 py-0.5 text-xs">
																<CollapsibleError content={part.content} />
															</div>
														);
													case "tool_use": {
														const pairedResult =
															nextPart?.type === "tool_result"
																? nextPart
																: undefined;
														return (
															<div key={key} className="px-5 py-0.5 text-xs">
																<ToolActivity
																	entry={part}
																	result={pairedResult}
																	index={i}
																/>
															</div>
														);
													}
													case "tool_result":
														return (
															<div key={key} className="px-5 py-0.5 text-xs">
																<ActivityItem entry={part} index={i} />
															</div>
														);
													case "permission":
														return (
															<PermissionDialog
																key={key}
																request={part.request}
																status={part.status}
																resolvedAnswers={part.answers}
																onAllow={(id) => respondPermission(id, true)}
																onDeny={(id) => respondPermission(id, false)}
																onAnswer={(id, answers) =>
																	respondPermission(id, true, {
																		...part.request.input,
																		answers,
																	})
																}
															/>
														);
												}
											})}
											{isLastAgentStreaming &&
												msg.parts.length > 0 &&
												msg.parts[msg.parts.length - 1].type === "tool_use" && (
													<ShimmerPlaceholder lines={2} />
												)}
										</div>
									);
								})}
								{showWaitingShimmer && <ShimmerPlaceholder />}
							</div>
						)}
					</div>
					<div className="shrink-0">
						{error && (
							<div className="px-2 pb-2">
								<div className="bg-destructive/10 text-destructive rounded-lg px-3 py-2 text-sm">
									{error}
								</div>
							</div>
						)}
						<MessageInput
							onSend={sendMessage}
							onInterrupt={interrupt}
							disabled={isInputDisabled}
							isStreaming={isStreaming}
							onCycleMode={cycleMode}
							mode={permissionMode}
							onModeChange={setPermissionMode}
						/>
					</div>
				</div>
			</Tabs>
		</div>
	);
}
