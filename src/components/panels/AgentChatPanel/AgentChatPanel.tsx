import { invoke } from "@tauri-apps/api/core";
import { History, X } from "lucide-react";
import React, {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { AgentStateIcon } from "@/components/ui/agent-state-icon";
import {
	Popover,
	PopoverContent,
	PopoverTrigger,
} from "@/components/ui/popover";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAgentChat } from "@/hooks/useAgentChat";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import { loadSlashCommands } from "@/hooks/useSlashCommands";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import type {
	ChatMessage,
	ImageAttachment,
	ImagePart,
	MentionReference,
	MessagePart,
} from "@/types/session";
import { getTextContent } from "@/types/session";
import {
	ActivityItem,
	CollapsibleError,
	TaskToolActivity,
	ToolActivity,
} from "./ActivityLog";
import { nextCodexPermissionMode } from "./CodexPermissionControl";
import type { MessageInputHandle } from "./MessageInput";
import { MessageInput } from "./MessageInput";
import { MODES } from "./ModeSelector";
import { PermissionDialog } from "./PermissionDialog";
import { ShimmerPlaceholder } from "./ShimmerPlaceholder";
import { StreamMessage } from "./StreamMessage";
import { buildToolPairings, type TaskGroup } from "./toolPairing";
import { WorkflowPanel } from "./WorkflowPanel";

type SystemNotificationPart = Extract<
	MessagePart,
	{ type: "system_notification" }
>;

function SystemNotificationItem({ part }: { part: SystemNotificationPart }) {
	const isInProgress = part.status === "in_progress";
	return (
		<div className="px-5 py-0.5 text-xs text-muted-foreground">
			<span className={isInProgress ? "animate-pulse" : ""}>
				{isInProgress ? "⏳ " : part.status === "error" ? "❌ " : "✓ "}
				{part.label}
			</span>
			{part.detail && <span className="ml-1 opacity-70">({part.detail})</span>}
		</div>
	);
}

interface AgentMessagePartsProps {
	msg: ChatMessage;
	isLastAgentStreaming: boolean;
	worktreePath: string;
	respondPermission: (
		id: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
}

const AgentMessageParts = React.memo(function AgentMessageParts({
	msg,
	isLastAgentStreaming,
	worktreePath,
	respondPermission,
}: AgentMessagePartsProps) {
	const { pairedResults, skippedResultIndices, taskGroups, taskChildIndices } =
		useMemo(() => buildToolPairings(msg.parts), [msg.parts]);

	const {
		backgroundCompletionMap,
		runningBackgroundTasks,
		backgroundToolUseIndices,
	} = useMemo(() => {
		const completionMap = new Map<number, TaskGroup>();
		const running: TaskGroup[] = [];
		const bgIndices = new Set<number>();

		for (const [idx, group] of taskGroups) {
			if (!group.isBackground) continue;
			bgIndices.add(idx);
			if (group.isCompleted && group.completionStatusIndex !== undefined) {
				completionMap.set(group.completionStatusIndex, group);
			} else if (!group.isCompleted) {
				running.push(group);
			}
		}

		return {
			backgroundCompletionMap: completionMap,
			runningBackgroundTasks: running,
			backgroundToolUseIndices: bgIndices,
		};
	}, [taskGroups]);

	return (
		<>
			{/* biome-ignore lint/suspicious/useIterableCallbackReturn: switch is exhaustive for MessagePart */}
			{msg.parts.map((part, i) => {
				const key = `${msg.id}-p${i}`;

				// バックグラウンドタスクのtool_use位置はスキップ（最下部 or completion位置で表示）
				if (backgroundToolUseIndices.has(i)) return null;

				// 完了バックグラウンドタスク: completion status位置に表示
				// NOTE: taskChildIndices.has(i) より前に判定（completionStatusIndexはtaskChildIndicesに含まれるため）
				{
					const bgCompletedGroup = backgroundCompletionMap.get(i);
					if (bgCompletedGroup) {
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<TaskToolActivity
									group={bgCompletedGroup}
									parts={msg.parts}
									pairedResults={pairedResults}
									isStreaming={isLastAgentStreaming}
									basePath={worktreePath}
								/>
							</div>
						);
					}
				}

				if (taskChildIndices.has(i) || part.type === "task_status") return null;

				// フォアグラウンドタスク（既存動作）
				{
					const taskGroup = taskGroups.get(i);
					if (taskGroup) {
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<TaskToolActivity
									group={taskGroup}
									parts={msg.parts}
									pairedResults={pairedResults}
									isStreaming={isLastAgentStreaming}
									basePath={worktreePath}
								/>
							</div>
						);
					}
				}

				switch (part.type) {
					case "thinking":
						return null;
					case "text":
						return (
							// biome-ignore lint/a11y/useValidAriaRole: role is a component prop, not an ARIA role
							<StreamMessage key={key} content={part.content} role="agent" />
						);
					case "error":
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<CollapsibleError content={part.content} />
							</div>
						);
					case "tool_use": {
						const pairedResult = pairedResults.get(i);
						const isExecuting = isLastAgentStreaming && !pairedResult;
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<ToolActivity
									entry={part}
									result={pairedResult}
									index={i}
									isExecuting={isExecuting}
									basePath={worktreePath}
								/>
							</div>
						);
					}
					case "tool_result": {
						if (skippedResultIndices.has(i)) return null;
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<ActivityItem entry={part} index={i} />
							</div>
						);
					}
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
					case "system_notification":
						return <SystemNotificationItem key={key} part={part} />;
					case "image":
						return null;
				}
			})}
			{runningBackgroundTasks.map((group) => (
				<div
					key={`${msg.id}-bg-${group.toolUseId}`}
					className="px-5 py-0.5 text-xs"
				>
					<TaskToolActivity
						group={group}
						parts={msg.parts}
						pairedResults={pairedResults}
						isStreaming={isLastAgentStreaming}
						basePath={worktreePath}
					/>
				</div>
			))}
		</>
	);
});

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

export function AgentChatPanel({
	worktreePath,
	registerDropZone,
	sendMessageRef,
}: AgentChatPanelProps) {
	const { workflowState } = useWorkflowState(worktreePath);
	const workflowApprovalChatSessionId =
		workflowState?.state.type === "waiting_approval"
			? (workflowState.currentSessionId ?? workflowState.chatSessionId)
			: null;
	const {
		sessions,
		orderedSessions,
		closedSessions,
		activeSession,
		isStreaming,
		activityStatus,
		error,
		permissionMode,
		sessionAgentStates,
		sendMessage,
		interrupt,
		selectSession,
		openWorkflowStepSession,
		closeSession,
		restoreSession,
		createNewSession,
		reorderSessions,
		setPermissionMode,
		respondPermission,
		refreshSessions,
		refreshClosedSessions,
		availableModels,
		selectedModel,
		setModel,
		backends,
		selectedBackendId,
		setBackend,
	} = useAgentChat(worktreePath, workflowApprovalChatSessionId);
	const knownWorkflowSessionIds = useMemo(() => {
		return new Set(sessions.map((session) => session.id));
	}, [sessions]);
	const workflowStateUpdatedAt = workflowState?.updatedAt;

	useEffect(() => {
		const workflowSessionIds = [
			workflowState?.chatSessionId,
			workflowState?.currentSessionId,
		].filter((id): id is string => Boolean(id));

		if (
			workflowSessionIds.some(
				(sessionId) => !knownWorkflowSessionIds.has(sessionId),
			)
		) {
			refreshSessions();
		}
	}, [
		workflowState?.chatSessionId,
		workflowState?.currentSessionId,
		knownWorkflowSessionIds,
		refreshSessions,
	]);

	useEffect(() => {
		if (workflowStateUpdatedAt == null) return;
		refreshSessions({ reconcileActiveSession: true });
		refreshClosedSessions();
	}, [workflowStateUpdatedAt, refreshSessions, refreshClosedSessions]);

	// Expose sendMessage to parent via ref (without images parameter)
	useEffect(() => {
		if (sendMessageRef) {
			sendMessageRef.current = (
				content: string,
				mentions?: MentionReference[],
			) => sendMessage(content, undefined, mentions);
		}
		return () => {
			if (sendMessageRef) {
				sendMessageRef.current = null;
			}
		};
	}, [sendMessage, sendMessageRef]);

	const [historyOpen, setHistoryOpen] = useState(false);
	const draggedSessionIdRef = useRef<string | null>(null);
	const isDraggingRef = useRef(false);

	const messageInputRef = useRef<MessageInputHandle>(null);
	const [isFileDragOver, setIsFileDragOver] = useState(false);
	const isFileDragOverRef = useRef(false);

	const scrollRef = useRef<HTMLDivElement>(null);
	const scrollAnchorRef = useRef<HTMLDivElement>(null);
	const lastMessageCount = useRef(0);
	const isNearBottomRef = useRef(true);

	// Load slash commands from filesystem on mount
	useEffect(() => {
		loadSlashCommands(worktreePath).catch((e) =>
			console.error("Failed to load slash commands:", e),
		);
	}, [worktreePath]);

	// Register agent drop zone for native file drop (image attachment)
	const agentDropZoneRef = useRef<HTMLDivElement>(null);
	const handleAgentDrop = useCallback(async (paths: string[]) => {
		isFileDragOverRef.current = false;
		setIsFileDragOver(false);
		try {
			const attachments = await invoke<ImageAttachment[]>(
				"prepare_image_attachments_from_paths",
				{ paths },
			);
			if (attachments.length > 0) {
				messageInputRef.current?.addImageAttachments(attachments);
			}
		} catch (e) {
			console.error("Failed to process dropped images:", e);
		}
	}, []);
	useEffect(() => {
		const el = agentDropZoneRef.current;
		if (el) {
			registerDropZone("agent", el, handleAgentDrop);
		}
		return () => {
			registerDropZone("agent", null);
		};
	}, [registerDropZone, handleAgentDrop]);

	const handleFileDragOver = useCallback((e: React.DragEvent) => {
		if (e.dataTransfer.types.includes("Files")) {
			e.preventDefault();
			e.dataTransfer.dropEffect = "copy";
			isFileDragOverRef.current = true;
			setIsFileDragOver(true);
		}
	}, []);

	const handleFileDragLeave = useCallback((e: React.DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			isFileDragOverRef.current = false;
			setIsFileDragOver(false);
		}
	}, []);

	// Track scroll position via onScroll handler
	const handleScroll = useCallback(() => {
		const el = scrollRef.current;
		if (!el) return;
		isNearBottomRef.current =
			el.scrollHeight - el.scrollTop - el.clientHeight < 100;
	}, []);

	// Derive streaming content tracking values
	const agentMessages = activeSession?.messages.filter(
		(m) => m.role === "agent",
	);
	const lastAgentMsg = agentMessages?.[agentMessages.length - 1];
	const lastAgentPartsLen = lastAgentMsg?.parts.length ?? 0;
	const lastAgentContent = getTextContent(lastAgentMsg?.parts ?? []).length;

	const msgs = activeSession?.messages;
	const lastMsg = msgs?.[msgs.length - 1];
	const shimmerLineCount = useMemo(() => {
		if (!isStreaming || lastMsg?.role !== "agent") return 0;
		if (lastMsg.parts.length === 0) return 3;
		const lastPart = lastMsg.parts[lastMsg.parts.length - 1];
		switch (lastPart.type) {
			case "thinking":
			case "tool_use":
			case "tool_result":
			case "task_status":
			case "system_notification":
				return 2;
			case "text":
				return 1;
			default:
				return 0;
		}
	}, [isStreaming, lastMsg]);

	// Auto-scroll: anchor-based approach
	// biome-ignore lint/correctness/useExhaustiveDependencies: lastAgentContent/lastAgentPartsLen/shimmerLineCount triggers scroll on content growth
	useEffect(() => {
		const count = activeSession?.messages.length ?? 0;
		if (count > lastMessageCount.current) {
			// New message added → force scroll
			scrollAnchorRef.current?.scrollIntoView({ behavior: "instant" });
		} else if (isNearBottomRef.current) {
			// Content update → follow if near bottom
			scrollAnchorRef.current?.scrollIntoView({ behavior: "instant" });
		}
		lastMessageCount.current = count;
	}, [
		activeSession?.messages.length,
		lastAgentContent,
		lastAgentPartsLen,
		shimmerLineCount,
	]);

	const cycleMode = useCallback(() => {
		if (selectedBackendId === "codex") {
			setPermissionMode(nextCodexPermissionMode(permissionMode));
			return;
		}
		const currentIndex = MODES.findIndex((m) => m.value === permissionMode);
		const nextIndex = (currentIndex + 1) % MODES.length;
		setPermissionMode(MODES[nextIndex].value);
	}, [permissionMode, selectedBackendId, setPermissionMode]);

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
	const canChangeBackend =
		!!activeSession &&
		activeSession.messages.length === 0 &&
		!activeSession.agentSessionId &&
		!isStreaming;

	return (
		<div data-testid="agent-chat-panel" className="flex flex-col h-full">
			<Group orientation="horizontal" className="flex-1 min-h-0">
				<Panel defaultSize="60%" minSize="30%">
					<Tabs
						value={activeSession?.id ?? ""}
						onValueChange={selectSession}
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
											<AgentStateIcon
												state={sessionAgentStates.get(session.id)}
											/>
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
						{/* biome-ignore lint/a11y/noStaticElementInteractions: native file drop target */}
						<div
							ref={agentDropZoneRef}
							className="flex flex-col flex-1 min-h-0 relative"
							onDragOver={handleFileDragOver}
							onDragLeave={handleFileDragLeave}
						>
							{isFileDragOver && (
								<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none z-10">
									<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
										Drop image to attach
									</span>
								</div>
							)}
							<div
								ref={scrollRef}
								onScroll={handleScroll}
								className="flex-1 min-h-0 overflow-auto select-text"
							>
								{activeSession && (
									<div className="py-2">
										{activeSession.messages.map((msg, idx) => {
											if (msg.role !== "agent") {
												const textContent = getTextContent(msg.parts);
												const imageParts = msg.parts.filter(
													(p): p is ImagePart => p.type === "image",
												);
												return (
													<div key={msg.id}>
														<StreamMessage
															content={textContent}
															role={msg.role}
															images={
																imageParts.length > 0 ? imageParts : undefined
															}
															mentions={msg.mentions}
														/>
													</div>
												);
											}

											const isLastMsg =
												idx === activeSession.messages.length - 1;
											const isLastAgentStreaming = isStreaming && isLastMsg;

											return (
												<div key={msg.id}>
													<AgentMessageParts
														msg={msg}
														isLastAgentStreaming={isLastAgentStreaming}
														worktreePath={worktreePath}
														respondPermission={respondPermission}
													/>
												</div>
											);
										})}
										{shimmerLineCount > 0 && (
											<ShimmerPlaceholder lines={shimmerLineCount} />
										)}
										<div ref={scrollAnchorRef} />
									</div>
								)}
							</div>
							<div className="shrink-0">
								{activityStatus && (
									<div className="px-4 pb-1 text-xs text-muted-foreground animate-pulse truncate">
										{activityStatus.label}
									</div>
								)}
								{error && (
									<div className="px-2 pb-2">
										<div className="bg-destructive/10 text-destructive rounded-lg px-3 py-2 text-sm">
											{error}
										</div>
									</div>
								)}
								<MessageInput
									ref={messageInputRef}
									onSend={sendMessage}
									onInterrupt={interrupt}
									isStreaming={isStreaming}
									onCycleMode={cycleMode}
									mode={permissionMode}
									onModeChange={setPermissionMode}
									models={availableModels}
									currentModelId={selectedModel}
									onModelChange={setModel}
									backends={backends}
									currentBackendId={selectedBackendId}
									onBackendChange={setBackend}
									backendDisabled={!canChangeBackend}
									worktreePath={worktreePath}
								/>
							</div>
						</div>
					</Tabs>
				</Panel>
				<Separator className="w-px bg-border" />
				<Panel defaultSize="40%" minSize="20%">
					<WorkflowPanel
						workflowState={workflowState ?? null}
						worktreePath={worktreePath}
						chatSessionId={activeSession?.id ?? null}
						onSessionClick={openWorkflowStepSession}
						onCloseSession={closeSession}
					/>
				</Panel>
			</Group>
		</div>
	);
}
