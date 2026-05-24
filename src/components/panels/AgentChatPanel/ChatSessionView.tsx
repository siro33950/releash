import { invoke } from "@tauri-apps/api/core";
import React, {
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import { loadSlashCommands } from "@/hooks/useSlashCommands";
import type {
	BackendInfo,
	ChatMessage,
	ChatSession,
	ImageAttachment,
	ImagePart,
	MentionReference,
	MessagePart,
	ModelInfo,
	PermissionMode,
} from "@/types/session";
import { getTextContent } from "@/types/session";
import {
	ActivityItem,
	CollapsibleError,
	TaskToolActivity,
	ToolActivity,
} from "./ActivityLog";
import type { MessageInputHandle } from "./MessageInput";
import { MessageInput } from "./MessageInput";
import { MODES } from "./ModeSelector";
import { PermissionDialog } from "./PermissionDialog";
import { ShimmerPlaceholder } from "./ShimmerPlaceholder";
import { StreamMessage } from "./StreamMessage";
import { buildToolPairings, type TaskGroup } from "./toolPairing";

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

				if (backgroundToolUseIndices.has(i)) return null;

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

export interface ChatSessionViewProps {
	/** spec issues-1023: 表示対象の完全 ChatSession。未選択時は親で出し分け。 */
	session: ChatSession;
	isStreaming: boolean;
	activityStatus: { label: string } | null;
	error: string | null;
	permissionMode: PermissionMode;
	availableModels: ModelInfo[];
	selectedModel: string | null;
	backends: BackendInfo[];
	selectedBackendId: string | null;
	canChangeBackend: boolean;
	worktreePath: string;
	/** メッセージ送信。session id へのバインドは親側で行う。 */
	onSend: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
	) => Promise<void>;
	onInterrupt: () => void;
	onPermissionModeChange: (mode: PermissionMode) => void;
	onModelChange: (modelId: string | null) => void;
	onBackendChange: (backendId: string | null) => void;
	onRespondPermission: (
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
	/**
	 * spec issues-1023: 画像 drop の登録 zone。AgentChatPanel = "agent"、
	 * Workflow panel = 別 zone を指定する想定。未指定なら drop 受付なし。
	 */
	registerDropZone?: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	dropZoneName?: DropZoneType;
	/** mount 直後に slash commands を読み込むかどうか（default: true）。 */
	loadSlashCommandsOnMount?: boolean;
	/**
	 * 親から sendMessage を呼び出すための ref（任意）。AgentChatPanel が外部に
	 * sendMessageRef を公開しているため互換のために提供する。
	 */
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
}

/**
 * spec issues-1023: AgentChatPanel と WorkflowView の双方から再利用される
 * 単一 session 用の chat view。message stream + activity status + error + MessageInput
 * までを内包する。tab bar / session 切替 UI は本コンポーネントは持たない（親側責務）。
 */
export function ChatSessionView({
	session,
	isStreaming,
	activityStatus,
	error,
	permissionMode,
	availableModels,
	selectedModel,
	backends,
	selectedBackendId,
	canChangeBackend,
	worktreePath,
	onSend,
	onInterrupt,
	onPermissionModeChange,
	onModelChange,
	onBackendChange,
	onRespondPermission,
	registerDropZone,
	dropZoneName,
	loadSlashCommandsOnMount = true,
	sendMessageRef,
}: ChatSessionViewProps) {
	const messageInputRef = useRef<MessageInputHandle>(null);
	const [isFileDragOver, setIsFileDragOver] = useState(false);
	const isFileDragOverRef = useRef(false);

	const scrollRef = useRef<HTMLDivElement>(null);
	const scrollAnchorRef = useRef<HTMLDivElement>(null);
	const lastMessageCount = useRef(0);
	const isNearBottomRef = useRef(true);

	// Load slash commands from filesystem on mount
	useEffect(() => {
		if (!loadSlashCommandsOnMount) return;
		loadSlashCommands(worktreePath).catch((e) =>
			console.error("Failed to load slash commands:", e),
		);
	}, [worktreePath, loadSlashCommandsOnMount]);

	// Register agent drop zone for native file drop (image attachment)
	const dropZoneRef = useRef<HTMLDivElement>(null);
	const handleDrop = useCallback(async (paths: string[]) => {
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
		if (!registerDropZone || !dropZoneName) return;
		const el = dropZoneRef.current;
		if (el) {
			registerDropZone(dropZoneName, el, handleDrop);
		}
		return () => {
			registerDropZone(dropZoneName, null);
		};
	}, [registerDropZone, dropZoneName, handleDrop]);

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
	const agentMessages = session.messages.filter((m) => m.role === "agent");
	const lastAgentMsg = agentMessages[agentMessages.length - 1];
	const lastAgentPartsLen = lastAgentMsg?.parts.length ?? 0;
	const lastAgentContent = getTextContent(lastAgentMsg?.parts ?? []).length;

	const msgs = session.messages;
	const lastMsg = msgs[msgs.length - 1];
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
		const count = session.messages.length;
		if (count > lastMessageCount.current) {
			scrollAnchorRef.current?.scrollIntoView({ behavior: "instant" });
		} else if (isNearBottomRef.current) {
			scrollAnchorRef.current?.scrollIntoView({ behavior: "instant" });
		}
		lastMessageCount.current = count;
	}, [
		session.messages.length,
		lastAgentContent,
		lastAgentPartsLen,
		shimmerLineCount,
	]);

	const cycleMode = useCallback(() => {
		const currentIndex = MODES.findIndex((m) => m.value === permissionMode);
		const nextIndex = (currentIndex + 1) % MODES.length;
		onPermissionModeChange(MODES[nextIndex].value);
	}, [permissionMode, onPermissionModeChange]);

	// Expose sendMessage to parent via ref (without images parameter)
	useEffect(() => {
		if (sendMessageRef) {
			sendMessageRef.current = (
				content: string,
				mentions?: MentionReference[],
			) => onSend(content, undefined, mentions);
		}
		return () => {
			if (sendMessageRef) {
				sendMessageRef.current = null;
			}
		};
	}, [onSend, sendMessageRef]);

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: native file drop target
		<div
			ref={dropZoneRef}
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
				<div className="py-2">
					{session.messages.map((msg, idx) => {
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
										images={imageParts.length > 0 ? imageParts : undefined}
										mentions={msg.mentions}
									/>
								</div>
							);
						}

						const isLastMsg = idx === session.messages.length - 1;
						const isLastAgentStreaming = isStreaming && isLastMsg;

						return (
							<div key={msg.id}>
								<AgentMessageParts
									msg={msg}
									isLastAgentStreaming={isLastAgentStreaming}
									worktreePath={worktreePath}
									respondPermission={onRespondPermission}
								/>
							</div>
						);
					})}
					{shimmerLineCount > 0 && (
						<ShimmerPlaceholder lines={shimmerLineCount} />
					)}
					<div ref={scrollAnchorRef} />
				</div>
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
					onSend={onSend}
					onInterrupt={onInterrupt}
					isStreaming={isStreaming}
					onCycleMode={cycleMode}
					mode={permissionMode}
					onModeChange={onPermissionModeChange}
					models={availableModels}
					currentModelId={selectedModel}
					onModelChange={onModelChange}
					backends={backends}
					currentBackendId={selectedBackendId}
					onBackendChange={onBackendChange}
					backendDisabled={!canChangeBackend}
					worktreePath={worktreePath}
				/>
			</div>
		</div>
	);
}
