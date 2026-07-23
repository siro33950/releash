import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import {
	AlertTriangle,
	Brain,
	Check,
	ChevronDown,
	ChevronRight,
	ChevronUp,
	Code2,
	Copy,
	Search,
	X,
} from "lucide-react";
import React, {
	useCallback,
	useEffect,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { OlderMessageEvictionOptions } from "@/hooks/useAgentChat";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import { useOperationSupervision } from "@/hooks/useOperationSupervision";
import type { SessionFeedbackEntry } from "@/hooks/useSessionStore";
import type {
	AgentEditorContext,
	AgentEditorSelection,
	AgentStallObservation,
	BackendInfo,
	ChatMessage,
	ChatSession,
	DisplayImagePart,
	ImageAttachment,
	MentionReference,
	MessagePart,
	ModelInfo,
	PermissionMode,
	PermissionRequest,
	PlanMode,
	QueuedAgentTurn,
	SessionNotice,
	SlashCommand,
	TurnInterruption,
} from "@/types/session";
import { getTextContent } from "@/types/session";
import {
	ActivityItem,
	AgentErrorBlock,
	TaskToolActivity,
	ToolActivity,
	useActivityLogSessionScope,
} from "./ActivityLog";
import type { MessageInputHandle } from "./MessageInput";
import { MessageInput } from "./MessageInput";
import { MODES } from "./ModeSelector";
import { PermissionDialog } from "./PermissionDialog";
import { SessionFeedbackBanners } from "./SessionFeedbackBanners";
import { ShimmerPlaceholder } from "./ShimmerPlaceholder";
import {
	AgentMarkdown,
	formatMessageTime,
	MessageCopyButton,
	StreamMessage,
} from "./StreamMessage";
import { buildToolPairings, type TaskGroup } from "./toolPairing";

type SystemNotificationPart = Extract<
	MessagePart,
	{ type: "system_notification" }
>;
type TodoListSnapshotPart = Extract<
	MessagePart,
	{ type: "todo_list_snapshot" }
>;
type PermissionPart = Extract<MessagePart, { type: "permission" }>;
type RespondPermission = (
	id: string,
	allow: boolean,
	updatedInput?: Record<string, unknown>,
) => void;

interface ThreadSearchMatch {
	messageId: string;
	matchIndex: number;
}

interface NativeCommandNotice {
	kind: "info" | "error";
	title: string;
	detail?: string;
	taskItems?: AgentTaskListReport["items"];
}

interface AgentTaskListReport {
	title: string;
	detail: string;
	activeCount: number;
	completedCount: number;
	totalCount: number;
	items: Array<{
		toolUseId: string;
		label: string;
		status: string;
		background: boolean;
	}>;
}

function taskListRevision(messages: ChatMessage[]): string {
	return messages
		.map((message) => {
			const parts = message.parts ?? [];
			const partRevision = parts
				.map((part) => {
					switch (part.type) {
						case "tool_use":
							return [
								part.type,
								part.id,
								part.tool,
								JSON.stringify(part.input ?? null),
							].join(":");
						case "tool_result":
							return [
								part.type,
								part.toolUseId ?? "",
								part.isError ? "error" : "ok",
							].join(":");
						case "task_status":
							return [
								part.type,
								part.taskToolUseId,
								part.status,
								part.description ?? part.summary ?? "",
							].join(":");
						default:
							return part.type;
					}
				})
				.join(",");
			return `${message.id}:${message.role}:${partRevision}`;
		})
		.join("|");
}

function isPrependOnly(previousIds: string[], nextIds: string[]): boolean {
	if (previousIds.length === 0 || nextIds.length <= previousIds.length) {
		return false;
	}
	const offset = nextIds.length - previousIds.length;
	return previousIds.every((id, index) => nextIds[index + offset] === id);
}

function isTailAppend(previousIds: string[], nextIds: string[]): boolean {
	if (nextIds.length <= previousIds.length) return false;
	if (previousIds.length === 0) return true;
	return previousIds.every((id, index) => nextIds[index] === id);
}

function shouldTailFollowMessageChange(
	previousIds: string[],
	nextIds: string[],
	isNearBottom: boolean,
): boolean {
	if (isPrependOnly(previousIds, nextIds)) return false;
	if (isTailAppend(previousIds, nextIds)) return true;
	return isNearBottom;
}

interface AgentPromptSuggestion {
	text: string;
	source: string;
}

const LOAD_OLDER_SCROLL_TOP_PX = 80;
const AGENT_MESSAGE_ESTIMATED_HEIGHT_PX = 112;
const HUMAN_MESSAGE_ESTIMATED_HEIGHT_PX = 72;
const SHIMMER_ESTIMATED_HEIGHT_PX = 56;

function estimateMessageRowSize(message: ChatMessage | undefined): number {
	if (!message) return HUMAN_MESSAGE_ESTIMATED_HEIGHT_PX;
	return message.role === "agent"
		? AGENT_MESSAGE_ESTIMATED_HEIGHT_PX
		: HUMAN_MESSAGE_ESTIMATED_HEIGHT_PX;
}

function isEditableShortcutTarget(target: EventTarget | null): boolean {
	if (!(target instanceof HTMLElement)) return false;
	const tagName = target.tagName.toLowerCase();
	return (
		tagName === "input" ||
		tagName === "textarea" ||
		tagName === "select" ||
		target.isContentEditable
	);
}

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

function latestTodoListSnapshot(
	messages: ChatMessage[],
): TodoListSnapshotPart | null {
	for (
		let messageIndex = messages.length - 1;
		messageIndex >= 0;
		messageIndex--
	) {
		const message = messages[messageIndex];
		if (!message) continue;
		for (
			let partIndex = message.parts.length - 1;
			partIndex >= 0;
			partIndex--
		) {
			const part = message.parts[partIndex];
			if (part?.type === "todo_list_snapshot") return part;
		}
	}
	return null;
}

function TodoListFooter({
	snapshot,
}: {
	snapshot: TodoListSnapshotPart | null;
}) {
	const [isExpanded, setIsExpanded] = useState(false);
	if (!snapshot || snapshot.items.length === 0) return null;

	const completedCount = snapshot.items.filter((item) => item.completed).length;
	const visibleItems = isExpanded ? snapshot.items : snapshot.items.slice(0, 3);
	const visibleItemsWithKeys = visibleItems.map((item, index) => ({
		item,
		key: `${item.completed ? "done" : "todo"}-${index}-${item.text}`,
	}));

	return (
		<div className="px-3 pb-2">
			<div className="rounded border border-border bg-background px-3 py-2 text-xs">
				<button
					type="button"
					className="flex w-full min-w-0 items-center gap-2 text-left text-muted-foreground hover:text-foreground"
					aria-expanded={isExpanded}
					onClick={() => setIsExpanded((current) => !current)}
				>
					<ChevronRight
						className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
					/>
					<span className="shrink-0 font-medium text-foreground">TODO</span>
					<span className="shrink-0 tabular-nums">
						{completedCount}/{snapshot.items.length} completed
					</span>
					<span className="min-w-0 truncate">
						{snapshot.items
							.map((item) => `${item.completed ? "[x]" : "[ ]"} ${item.text}`)
							.join("  ")}
					</span>
				</button>
				{isExpanded && (
					<div className="mt-2 space-y-1 pl-5">
						{visibleItemsWithKeys.map(({ item, key }) => (
							<div key={key} className="flex min-w-0 items-start gap-2">
								<span
									className={
										item.completed
											? "mt-0.5 inline-flex size-3.5 shrink-0 items-center justify-center rounded-sm border border-muted-foreground/50 bg-muted text-muted-foreground"
											: "mt-0.5 inline-flex size-3.5 shrink-0 rounded-sm border border-muted-foreground/50"
									}
									aria-hidden="true"
								>
									{item.completed && <Check className="size-3" />}
								</span>
								<span
									className={
										item.completed
											? "min-w-0 break-words text-muted-foreground line-through"
											: "min-w-0 break-words text-foreground"
									}
								>
									{item.text}
								</span>
							</div>
						))}
						{snapshot.items.length > visibleItems.length && (
							<div className="text-muted-foreground">
								+{snapshot.items.length - visibleItems.length} more
							</div>
						)}
					</div>
				)}
			</div>
		</div>
	);
}

function ThinkingPart({
	content,
	isStreaming,
	showContent,
}: {
	content: string;
	isStreaming: boolean;
	showContent: boolean;
}) {
	const [isExpanded, setIsExpanded] = useState(true);
	const [hasManualToggle, setHasManualToggle] = useState(false);
	const trimmed = content.trim();
	useEffect(() => {
		if (isStreaming) {
			setIsExpanded(true);
			setHasManualToggle(false);
			return;
		}
		if (!hasManualToggle) {
			setIsExpanded(false);
		}
	}, [isStreaming, hasManualToggle]);
	if (!trimmed) return null;

	return (
		<div className="px-5 py-1 text-xs" data-testid="thinking-block">
			<button
				type="button"
				className="flex min-w-0 items-center gap-1.5 text-muted-foreground/75 hover:text-foreground/85"
				onClick={() => {
					setHasManualToggle(true);
					setIsExpanded((current) => !current);
				}}
				aria-expanded={isExpanded}
			>
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
				<Brain className="size-3 shrink-0" />
				<span className={isStreaming ? "animate-pulse" : ""}>Thinking</span>
			</button>
			{showContent && isExpanded && (
				<div className="mt-1 ml-5 max-h-56 overflow-y-auto rounded border border-border/60 bg-muted/25 px-2.5 py-2 text-muted-foreground">
					<AgentMarkdown content={trimmed} />
				</div>
			)}
		</div>
	);
}

function AgentMessageMeta({
	msg,
	isStreaming,
	interruption,
}: {
	msg: ChatMessage;
	isStreaming: boolean;
	interruption?: TurnInterruption | null;
}) {
	if (isStreaming) return null;
	const formattedTime = formatMessageTime(msg.timestamp);
	const copyableText = getTextContent(msg.parts).trim();
	if (!formattedTime && !copyableText && !interruption) return null;
	const interruptionReason = interruption
		? interruption.reason === "session_closed"
			? "Session closed"
			: interruption.reason
		: null;

	return (
		<div className="flex items-center gap-1 px-5 pb-1 text-[11px] text-muted-foreground">
			{interruptionReason && (
				<span
					className="rounded-full border border-border bg-muted/60 px-2 py-0.5"
					data-testid="turn-interruption-chip"
				>
					Interrupted: {interruptionReason}
				</span>
			)}
			{formattedTime && <span>{formattedTime}</span>}
			{copyableText && (
				<MessageCopyButton
					content={copyableText}
					ariaLabel="Copy agent message"
				/>
			)}
		</div>
	);
}

interface AgentMessagePartsProps {
	msg: ChatMessage;
	sessionId: string;
	isLastAgentStreaming: boolean;
	worktreePath: string;
	showThinkingContent: boolean;
	rawScrollback: boolean;
	onOpenDiffFile?: (filePath: string) => void;
	permissionVisibilityRoot?: Element | null;
	respondPermission: RespondPermission;
}

function PermissionDialogSlot({
	request,
	status,
	sessionId,
	resolvedAnswers,
	worktreePath,
	onOpenDiffFile,
	visibilityRoot,
	respondPermission,
}: {
	request: PermissionRequest;
	status: PermissionPart["status"];
	sessionId: string;
	resolvedAnswers?: Record<string, string | string[]>;
	worktreePath: string;
	onOpenDiffFile?: (filePath: string) => void;
	visibilityRoot?: Element | null;
	respondPermission: RespondPermission;
}) {
	return (
		<PermissionDialog
			request={request}
			status={status}
			sessionId={sessionId}
			resolvedAnswers={resolvedAnswers}
			worktreePath={worktreePath}
			onOpenDiffFile={onOpenDiffFile}
			visibilityRoot={visibilityRoot}
			onAllow={(id, updatedInput) => respondPermission(id, true, updatedInput)}
			onDeny={(id) => respondPermission(id, false)}
			onAnswer={(id, answers) =>
				respondPermission(id, true, {
					...(request.input ?? {}),
					answers,
				})
			}
		/>
	);
}

const AgentMessageParts = React.memo(function AgentMessageParts({
	msg,
	sessionId,
	isLastAgentStreaming,
	worktreePath,
	showThinkingContent,
	rawScrollback,
	onOpenDiffFile,
	permissionVisibilityRoot,
	respondPermission,
}: AgentMessagePartsProps) {
	const {
		pairedResultGroups,
		skippedResultIndices,
		taskGroups,
		taskChildIndices,
	} = useMemo(() => buildToolPairings(msg.parts), [msg.parts]);

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
									pairedResultGroups={pairedResultGroups}
									isStreaming={isLastAgentStreaming}
									basePath={worktreePath}
									sessionId={sessionId}
									onOpenDiffFile={onOpenDiffFile}
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
									pairedResultGroups={pairedResultGroups}
									isStreaming={isLastAgentStreaming}
									basePath={worktreePath}
									sessionId={sessionId}
									onOpenDiffFile={onOpenDiffFile}
								/>
							</div>
						);
					}
				}

				switch (part.type) {
					case "thinking":
						return (
							<ThinkingPart
								key={key}
								content={part.content}
								isStreaming={isLastAgentStreaming}
								showContent={showThinkingContent}
							/>
						);
					case "text":
						return (
							// biome-ignore lint/a11y/useValidAriaRole: role is a component prop, not an ARIA role
							<StreamMessage
								key={key}
								content={part.content}
								role="agent"
								rawMode={rawScrollback}
							/>
						);
					case "error":
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<AgentErrorBlock content={part.content} />
							</div>
						);
					case "tool_use": {
						const pairedResultGroup = pairedResultGroups.get(i);
						const isExecuting =
							isLastAgentStreaming && !pairedResultGroup?.length;
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<ToolActivity
									entry={part}
									results={pairedResultGroup}
									index={i}
									isExecuting={isExecuting}
									basePath={worktreePath}
									sessionId={sessionId}
									onOpenDiffFile={onOpenDiffFile}
								/>
							</div>
						);
					}
					case "tool_result": {
						if (skippedResultIndices.has(i)) return null;
						return (
							<div key={key} className="px-5 py-0.5 text-xs">
								<ActivityItem entry={part} index={i} sessionId={sessionId} />
							</div>
						);
					}
					case "permission":
						return (
							<PermissionDialogSlot
								key={key}
								request={part.request}
								status={part.status}
								sessionId={sessionId}
								resolvedAnswers={part.answers}
								worktreePath={worktreePath}
								onOpenDiffFile={onOpenDiffFile}
								visibilityRoot={permissionVisibilityRoot}
								respondPermission={respondPermission}
							/>
						);
					case "system_notification":
						return <SystemNotificationItem key={key} part={part} />;
					case "todo_list_snapshot":
						return null;
					case "image":
					case "image_ref":
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
						pairedResultGroups={pairedResultGroups}
						isStreaming={isLastAgentStreaming}
						basePath={worktreePath}
						sessionId={sessionId}
						onOpenDiffFile={onOpenDiffFile}
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
	isInterrupting: boolean;
	activityStatus: { label: string } | null;
	error: string | null;
	onDismissError: () => void;
	permissionMode: PermissionMode;
	planMode: PlanMode;
	availableModels: ModelInfo[];
	backends: BackendInfo[];
	selectedModel: string;
	pendingPermission: PermissionRequest | null;
	pendingQueue: QueuedAgentTurn[];
	queuePaused: boolean;
	stallObservation?: AgentStallObservation | null;
	notice?: SessionNotice | null;
	feedback?: SessionFeedbackEntry[];
	onDismissFeedback?: (entry: SessionFeedbackEntry) => void | Promise<void>;
	onRetryFeedback?: (entry: SessionFeedbackEntry) => void | Promise<void>;
	hasMoreFeedback?: boolean;
	onLoadMoreFeedback?: () => void | Promise<void>;
	runtimeSlashCommands?: SlashCommand[];
	selectedBackendId: string | null;
	canChangeBackend: boolean;
	worktreePath: string;
	activeEditorPath?: string | null;
	openEditorPaths?: string[];
	activeEditorSelection?: AgentEditorSelection | null;
	/** メッセージ送信。session id へのバインドは親側で行う。 */
	onSend: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
		options?: {
			activateNewSession?: boolean;
			forkNewSession?: boolean;
			editorContext?: AgentEditorContext;
		},
	) => Promise<boolean>;
	onInterrupt: () => void;
	onCancelQueuedTurn: (queuedTurnId?: string | null) => Promise<void>;
	onResumeQueue: () => Promise<void>;
	onLoadOlderMessages?: () => Promise<void>;
	onEvictOlderMessages?: (
		options: OlderMessageEvictionOptions,
	) => void | Promise<void>;
	onPermissionModeChange: (mode: PermissionMode) => void;
	onPlanModeChange: (enabled: PlanMode) => void;
	onModelChange: (modelId: string) => void;
	onRespondPermission: (
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
	onOpenDiffFile?: (filePath: string) => void;
	/**
	 * 画像 drop の登録 zone。NodeContentView の Session content は "agent" を使う。
	 * 未指定なら drop 受付なし。
	 */
	registerDropZone?: (
		zone: DropZoneType,
		element: HTMLElement | null,
		onDrop?: (paths: string[]) => void,
	) => void;
	dropZoneName?: DropZoneType;
	/**
	 * 親から選択中Node SessionへsendMessageを呼び出すためのref（任意）。
	 */
	sendMessageRef?: React.MutableRefObject<
		((content: string, mentions?: MentionReference[]) => Promise<void>) | null
	>;
}

/**
 * BoundSessionChatから再利用される単一session用のchat view。
 * message stream + activity status + error + MessageInput
 * までを内包する。tab bar / session 切替 UI は本コンポーネントは持たない（親側責務）。
 */
export function ChatSessionView({
	session,
	isStreaming,
	isInterrupting,
	activityStatus,
	error,
	onDismissError,
	permissionMode,
	planMode,
	availableModels,
	backends,
	selectedModel,
	pendingPermission,
	pendingQueue,
	queuePaused,
	stallObservation,
	notice,
	feedback = [],
	onDismissFeedback,
	onRetryFeedback,
	hasMoreFeedback = false,
	onLoadMoreFeedback,
	runtimeSlashCommands = [],
	selectedBackendId,
	canChangeBackend,
	worktreePath,
	activeEditorPath,
	openEditorPaths,
	activeEditorSelection,
	onSend,
	onInterrupt,
	onCancelQueuedTurn,
	onResumeQueue,
	onLoadOlderMessages,
	onEvictOlderMessages,
	onPermissionModeChange,
	onPlanModeChange,
	onModelChange,
	onRespondPermission,
	onOpenDiffFile,
	registerDropZone,
	dropZoneName,
	sendMessageRef,
}: ChatSessionViewProps) {
	useActivityLogSessionScope(session.id);
	const supervision = useOperationSupervision(session.id);
	const messageInputRef = useRef<MessageInputHandle>(null);
	const [isFileDragOver, setIsFileDragOver] = useState(false);
	const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
		"idle",
	);
	const [promptSuggestion, setPromptSuggestion] = useState<string | null>(null);
	const [threadSearchState, setThreadSearchState] = useState({
		open: false,
		requestId: 0,
	});
	const [searchQuery, setSearchQuery] = useState("");
	const [searchMatches, setSearchMatches] = useState<ThreadSearchMatch[]>([]);
	const [activeSearchIndex, setActiveSearchIndex] = useState(0);
	const [showThinkingContent, setShowThinkingContent] = useState(true);
	const [rawScrollback, setRawScrollback] = useState(false);
	const [nativeCommandNotice, setNativeCommandNotice] =
		useState<NativeCommandNotice | null>(null);
	const stallIdleLabel = useMemo(() => {
		if (!stallObservation) return null;
		const minutes = Math.floor(stallObservation.idleSecs / 60);
		const seconds = stallObservation.idleSecs % 60;
		if (minutes <= 0) return `${seconds}s`;
		if (seconds === 0) return `${minutes}m`;
		return `${minutes}m ${seconds}s`;
	}, [stallObservation]);
	const [, setSelectedPermissionProfileId] = useState<string | null>(
		session.permissionProfileId ?? null,
	);

	useEffect(() => {
		setSelectedPermissionProfileId(session.permissionProfileId ?? null);
	}, [session.permissionProfileId]);
	const isFileDragOverRef = useRef(false);

	const currentEditorContext = useMemo<AgentEditorContext | undefined>(() => {
		const active = activeEditorPath?.trim() || null;
		const open = Array.from(
			new Set(
				(openEditorPaths ?? []).map((path) => path.trim()).filter(Boolean),
			),
		);
		const selection = activeEditorSelection ?? null;
		if (!active && open.length === 0 && !selection) return undefined;
		return {
			activeEditorPath: active,
			openEditorPaths: open,
			...(selection ? { selection } : {}),
		};
	}, [activeEditorPath, openEditorPaths, activeEditorSelection]);

	const rootRef = useRef<HTMLDivElement>(null);
	const dropZoneRef = useRef<HTMLDivElement>(null);
	const searchInputRef = useRef<HTMLInputElement>(null);
	const messageRefs = useRef(new Map<string, HTMLDivElement>());
	const scrollRef = useRef<HTMLDivElement>(null);
	const [permissionVisibilityRoot, setPermissionVisibilityRoot] =
		useState<HTMLDivElement | null>(null);
	const setScrollElement = useCallback((node: HTMLDivElement | null) => {
		scrollRef.current = node;
		setPermissionVisibilityRoot(node);
	}, []);
	const pendingScrollCompensationRef = useRef(0);
	const lastMessageSnapshotRef = useRef<{
		sessionId: string;
		messageIds: string[];
	}>({ sessionId: "", messageIds: [] });
	const lastEvictionCheckMessageLengthRef = useRef(session.messages.length);
	const isNearBottomRef = useRef(true);
	const pendingTopLoadEvictionRef = useRef<{
		firstMessageId: string | null;
	} | null>(null);
	const lastTaskListRevisionRef = useRef<string | null>(null);
	const currentTaskListRevision = useMemo(
		() => taskListRevision(session.messages),
		[session.messages],
	);
	const messagePermissionRequestIds = useMemo(() => {
		const ids = new Set<string>();
		for (const message of session.messages) {
			for (const part of message.parts) {
				if (part.type === "permission" && part.request.id) {
					ids.add(part.request.id);
				}
			}
		}
		return ids;
	}, [session.messages]);
	const fallbackPendingPermission =
		pendingPermission && !messagePermissionRequestIds.has(pendingPermission.id)
			? pendingPermission
			: null;
	const fallbackPendingPermissionId = fallbackPendingPermission?.id ?? null;

	// Register agent drop zone for native file drop (image attachment)
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

	// Derive streaming content tracking values
	const agentMessages = session.messages.filter((m) => m.role === "agent");
	const lastAgentMsg = agentMessages[agentMessages.length - 1];
	const lastAgentPartsLen = lastAgentMsg?.parts.length ?? 0;
	const lastAgentContent = getTextContent(lastAgentMsg?.parts ?? []).length;

	const msgs = session.messages;
	const lastMsg = msgs[msgs.length - 1];
	const hasThinkingContent = useMemo(
		() =>
			session.messages.some((message) =>
				message.parts.some(
					(part) => part.type === "thinking" && part.content.trim().length > 0,
				),
			),
		[session.messages],
	);
	const activeSearchMatch = searchMatches[activeSearchIndex] ?? null;
	const isSearchOpen = threadSearchState.open;

	const openThreadSearch = useCallback(() => {
		setThreadSearchState((current) => ({
			open: true,
			requestId: current.requestId + 1,
		}));
	}, []);

	const closeThreadSearch = useCallback(() => {
		setThreadSearchState((current) => ({ ...current, open: false }));
	}, []);

	const toggleRawScrollback = useCallback(() => {
		setRawScrollback((current) => !current);
	}, []);

	const latestCopyableAgentText = useMemo(() => {
		for (let i = agentMessages.length - 1; i >= 0; i--) {
			const msg = agentMessages[i];
			if (isStreaming && lastMsg?.role === "agent" && msg.id === lastMsg.id) {
				continue;
			}
			const text = getTextContent(msg.parts);
			if (text.trim().length > 0) return text;
		}
		return "";
	}, [agentMessages, isStreaming, lastMsg]);
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
	const virtualRowCount =
		session.messages.length + (shimmerLineCount > 0 ? 1 : 0);
	const messageVirtualizer = useVirtualizer({
		count: virtualRowCount,
		getScrollElement: () => scrollRef.current,
		getItemKey: (index) => session.messages[index]?.id ?? "agent-shimmer",
		estimateSize: (index) => {
			if (index >= session.messages.length) return SHIMMER_ESTIMATED_HEIGHT_PX;
			return estimateMessageRowSize(session.messages[index]);
		},
		overscan: 8,
		initialRect: {
			width: 800,
			height: 800,
		},
	});

	const prefixOffsetForCount = useCallback(
		(count: number) => {
			if (count <= 0) return 0;
			const offset = messageVirtualizer.getOffsetForIndex(count, "start")?.[0];
			if (typeof offset === "number" && Number.isFinite(offset)) {
				return offset;
			}
			const virtualItem = messageVirtualizer
				.getVirtualItems()
				.find((item) => item.index === count);
			return virtualItem?.start ?? 0;
		},
		[messageVirtualizer],
	);

	const requestMessageEviction = useCallback(
		(allowTopWindow = false) => {
			const virtualItems = messageVirtualizer
				.getVirtualItems()
				.filter((item) => item.index < session.messages.length);
			const firstVirtualItem = virtualItems[0];
			if (!firstVirtualItem) return;
			if (!allowTopWindow && firstVirtualItem.index <= 0) return;
			void onEvictOlderMessages?.({
				oldestVisibleIndex: firstVirtualItem.index,
				onEvicted: ({ count }) => {
					pendingScrollCompensationRef.current += prefixOffsetForCount(count);
				},
			});
		},
		[
			session.messages.length,
			messageVirtualizer,
			onEvictOlderMessages,
			prefixOffsetForCount,
		],
	);

	const firstMessageId = session.messages[0]?.id ?? null;

	// Track scroll position via onScroll handler.
	const handleScroll = useCallback(() => {
		const el = scrollRef.current;
		if (!el) return;
		isNearBottomRef.current =
			el.scrollHeight - el.scrollTop - el.clientHeight < 100;
		if (el.scrollTop < LOAD_OLDER_SCROLL_TOP_PX) {
			pendingTopLoadEvictionRef.current = { firstMessageId };
			void onLoadOlderMessages?.();
			return;
		}
		pendingTopLoadEvictionRef.current = null;
		requestMessageEviction();
	}, [firstMessageId, onLoadOlderMessages, requestMessageEviction]);

	useEffect(() => {
		const previousLength = lastEvictionCheckMessageLengthRef.current;
		lastEvictionCheckMessageLengthRef.current = session.messages.length;
		if (session.messages.length <= previousLength) return;
		const pendingTopLoadEviction = pendingTopLoadEvictionRef.current;
		if (pendingTopLoadEviction) {
			const didPrependOlderMessages =
				firstMessageId !== pendingTopLoadEviction.firstMessageId;
			if (didPrependOlderMessages) {
				pendingTopLoadEvictionRef.current = null;
				requestMessageEviction(true);
				return;
			}
		}
		if (!isNearBottomRef.current) return;
		requestMessageEviction();
	}, [firstMessageId, requestMessageEviction, session.messages.length]);

	useLayoutEffect(() => {
		const compensation = pendingScrollCompensationRef.current;
		if (compensation <= 0) return;
		pendingScrollCompensationRef.current = 0;
		const el = scrollRef.current;
		if (!el) return;
		el.scrollTop = Math.max(0, el.scrollTop - compensation);
		isNearBottomRef.current =
			el.scrollHeight - el.scrollTop - el.clientHeight < 100;
	});

	// Auto-scroll: keep tail-following behavior while only mounted rows are in the DOM.
	// biome-ignore lint/correctness/useExhaustiveDependencies: lastAgentContent/lastAgentPartsLen/shimmerLineCount intentionally trigger tail follow during streaming growth.
	useEffect(() => {
		const nextMessageIds = session.messages.map((message) => message.id);
		const previousSnapshot = lastMessageSnapshotRef.current;
		const previousMessageIds =
			previousSnapshot.sessionId === session.id
				? previousSnapshot.messageIds
				: [];
		if (
			shouldTailFollowMessageChange(
				previousMessageIds,
				nextMessageIds,
				isNearBottomRef.current,
			)
		) {
			messageVirtualizer.scrollToIndex(Math.max(virtualRowCount - 1, 0), {
				align: "end",
			});
		}
		lastMessageSnapshotRef.current = {
			sessionId: session.id,
			messageIds: nextMessageIds,
		};
	}, [
		session.id,
		session.messages.length,
		lastAgentContent,
		lastAgentPartsLen,
		shimmerLineCount,
		virtualRowCount,
		messageVirtualizer,
	]);

	useEffect(() => {
		if (!fallbackPendingPermissionId || !isNearBottomRef.current) return;
		const el = scrollRef.current;
		if (!el) return;
		el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
		isNearBottomRef.current = true;
	}, [fallbackPendingPermissionId]);

	const promptSuggestionRequest = useMemo(
		() => ({
			chatSessionId: session.id,
			updatedAt: session.updatedAt,
		}),
		[session.id, session.updatedAt],
	);

	useEffect(() => {
		if (isStreaming) {
			setPromptSuggestion(null);
			return;
		}
		let cancelled = false;
		const { chatSessionId } = promptSuggestionRequest;
		invoke<AgentPromptSuggestion | null>("build_agent_prompt_suggestion", {
			chatSessionId,
		})
			.then((suggestion) => {
				if (cancelled) return;
				setPromptSuggestion(
					suggestion && typeof suggestion.text === "string"
						? suggestion.text
						: null,
				);
			})
			.catch(() => {
				if (!cancelled) setPromptSuggestion(null);
			});
		return () => {
			cancelled = true;
		};
	}, [isStreaming, promptSuggestionRequest]);

	const cycleMode = useCallback(() => {
		const currentIndex = MODES.findIndex((m) => m.value === permissionMode);
		const nextIndex = (currentIndex + 1) % MODES.length;
		setSelectedPermissionProfileId(null);
		onPermissionModeChange(MODES[nextIndex].value);
	}, [permissionMode, onPermissionModeChange]);

	const handlePermissionModeChange = useCallback(
		(mode: PermissionMode) => {
			setSelectedPermissionProfileId(null);
			onPermissionModeChange(mode);
		},
		[onPermissionModeChange],
	);

	const handleCopyLatestAgentText = useCallback(async () => {
		if (!latestCopyableAgentText) return;
		try {
			await navigator.clipboard.writeText(latestCopyableAgentText);
			setCopyState("copied");
		} catch (e) {
			console.error("Failed to copy latest agent response:", e);
			setCopyState("error");
		}
	}, [latestCopyableAgentText]);

	const handleComposerSend = useCallback(
		async (
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
		) => {
			return onSend(content, images, mentions, {
				editorContext: currentEditorContext,
			});
		},
		[onSend, currentEditorContext],
	);

	useEffect(() => {
		if (copyState === "idle") return;
		const id = window.setTimeout(() => setCopyState("idle"), 1200);
		return () => window.clearTimeout(id);
	}, [copyState]);

	useLayoutEffect(() => {
		const handleCopyShortcut = (event: KeyboardEvent) => {
			if (!event.ctrlKey || event.key.toLowerCase() !== "o") return;
			if (
				rootRef.current &&
				document.activeElement &&
				document.activeElement !== document.body &&
				!rootRef.current.contains(document.activeElement) &&
				isEditableShortcutTarget(document.activeElement)
			) {
				return;
			}
			event.preventDefault();
			void handleCopyLatestAgentText();
		};
		window.addEventListener("keydown", handleCopyShortcut);
		return () => window.removeEventListener("keydown", handleCopyShortcut);
	}, [handleCopyLatestAgentText]);

	useEffect(() => {
		if (activeSearchIndex < searchMatches.length) return;
		setActiveSearchIndex(Math.max(searchMatches.length - 1, 0));
	}, [activeSearchIndex, searchMatches.length]);

	useEffect(() => {
		const query = searchQuery.trim();
		if (!isSearchOpen || !query) {
			setSearchMatches([]);
			setActiveSearchIndex(0);
			return;
		}
		let cancelled = false;
		void invoke<ThreadSearchMatch[]>("search_agent_session_messages", {
			sessionId: session.id,
			query,
		})
			.then((matches) => {
				if (cancelled) return;
				setSearchMatches(matches);
				setActiveSearchIndex(0);
			})
			.catch((error) => {
				if (cancelled) return;
				console.error("Failed to search current thread", error);
				setSearchMatches([]);
				setActiveSearchIndex(0);
			});
		return () => {
			cancelled = true;
		};
	}, [isSearchOpen, searchQuery, session.id]);

	useEffect(() => {
		if (!threadSearchState.open) return;
		searchInputRef.current?.focus();
		searchInputRef.current?.select();
	}, [threadSearchState]);

	useEffect(() => {
		if (!activeSearchMatch) return;
		const index = session.messages.findIndex(
			(message) => message.id === activeSearchMatch.messageId,
		);
		if (index === -1) return;
		messageVirtualizer.scrollToIndex(index, { align: "center" });
		messageRefs.current.get(activeSearchMatch.messageId)?.scrollIntoView({
			behavior: "smooth",
			block: "center",
		});
	}, [activeSearchMatch, session.messages, messageVirtualizer]);

	useLayoutEffect(() => {
		const handleFindShortcut = (event: KeyboardEvent) => {
			if (
				!(event.metaKey || event.ctrlKey) ||
				event.key.toLowerCase() !== "f"
			) {
				return;
			}
			if (
				rootRef.current &&
				document.activeElement &&
				document.activeElement !== document.body &&
				!rootRef.current.contains(document.activeElement) &&
				isEditableShortcutTarget(document.activeElement)
			) {
				return;
			}
			event.preventDefault();
			openThreadSearch();
		};
		window.addEventListener("keydown", handleFindShortcut);
		return () => window.removeEventListener("keydown", handleFindShortcut);
	}, [openThreadSearch]);

	useLayoutEffect(() => {
		const handleThinkingShortcut = (event: KeyboardEvent) => {
			if (
				event.key !== "Tab" ||
				event.shiftKey ||
				event.metaKey ||
				event.ctrlKey ||
				event.altKey ||
				!hasThinkingContent ||
				isEditableShortcutTarget(event.target)
			) {
				return;
			}
			if (
				rootRef.current &&
				document.activeElement &&
				document.activeElement !== document.body &&
				!rootRef.current.contains(document.activeElement) &&
				isEditableShortcutTarget(document.activeElement)
			) {
				return;
			}
			event.preventDefault();
			setShowThinkingContent((current) => !current);
		};
		window.addEventListener("keydown", handleThinkingShortcut);
		return () => window.removeEventListener("keydown", handleThinkingShortcut);
	}, [hasThinkingContent]);

	const goToNextSearchMatch = useCallback(() => {
		setActiveSearchIndex((current) =>
			searchMatches.length === 0 ? 0 : (current + 1) % searchMatches.length,
		);
	}, [searchMatches.length]);

	const goToPreviousSearchMatch = useCallback(() => {
		setActiveSearchIndex((current) =>
			searchMatches.length === 0
				? 0
				: (current - 1 + searchMatches.length) % searchMatches.length,
		);
	}, [searchMatches.length]);

	const setRootElement = useCallback((node: HTMLDivElement | null) => {
		rootRef.current = node;
		dropZoneRef.current = node;
	}, []);

	const todoListSnapshot = useMemo(
		() => latestTodoListSnapshot(session.messages),
		[session.messages],
	);

	const showTaskList = useCallback(async () => {
		lastTaskListRevisionRef.current = currentTaskListRevision;
		try {
			const report = await invoke<AgentTaskListReport>(
				"build_agent_task_list_report",
				{
					chatSessionId: session.id,
				},
			);
			setNativeCommandNotice({
				kind: "info",
				title: report.title,
				detail: report.detail,
				taskItems: report.items,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Task list unavailable",
				detail: String(e),
			});
		}
	}, [currentTaskListRevision, session.id]);

	const toggleTaskList = useCallback(() => {
		if (nativeCommandNotice?.title.startsWith("Tasks:")) {
			lastTaskListRevisionRef.current = null;
			setNativeCommandNotice(null);
			return;
		}
		void showTaskList();
	}, [nativeCommandNotice?.title, showTaskList]);

	const isTaskListOpen =
		nativeCommandNotice?.title.startsWith("Tasks:") ?? false;
	useEffect(() => {
		if (!isTaskListOpen) return;
		if (lastTaskListRevisionRef.current === currentTaskListRevision) return;
		void showTaskList();
	}, [currentTaskListRevision, isTaskListOpen, showTaskList]);

	useLayoutEffect(() => {
		const handleTaskListShortcut = (event: KeyboardEvent) => {
			if (!event.ctrlKey || event.key.toLowerCase() !== "t") return;
			if (
				rootRef.current &&
				document.activeElement &&
				document.activeElement !== document.body &&
				!rootRef.current.contains(document.activeElement) &&
				isEditableShortcutTarget(document.activeElement)
			) {
				return;
			}
			event.preventDefault();
			toggleTaskList();
		};
		window.addEventListener("keydown", handleTaskListShortcut);
		return () => window.removeEventListener("keydown", handleTaskListShortcut);
	}, [toggleTaskList]);

	// Expose sendMessage to parent via ref (without images parameter)
	useEffect(() => {
		if (sendMessageRef) {
			sendMessageRef.current = async (
				content: string,
				mentions?: MentionReference[],
			) => {
				await handleComposerSend(content, undefined, mentions);
			};
		}
		return () => {
			if (sendMessageRef) {
				sendMessageRef.current = null;
			}
		};
	}, [handleComposerSend, sendMessageRef]);

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: native file drop target
		<div
			ref={setRootElement}
			className="flex flex-col flex-1 min-h-0 relative"
			onDragOver={handleFileDragOver}
			onDragLeave={handleFileDragLeave}
		>
			{(supervision.state.sendOperation?.latest_status.type ===
				"reconciliation_required" ||
				supervision.state.recovery.length > 0 ||
				supervision.state.migration ||
				supervision.state.shutdown ||
				supervision.state.shutdownOutcomeUnknown) && (
				<div className="border-b border-border bg-muted/40 px-3 py-2 text-xs">
					{supervision.state.sendOperation?.latest_status.type ===
						"reconciliation_required" && (
						<div>
							Accepted send requires reconciliation:{" "}
							{supervision.state.sendOperation.receipt.operation_id}
						</div>
					)}
					{supervision.state.migration && (
						<div>
							Local data migration: {supervision.state.migration.phase}
							{supervision.state.migration.safe_failure
								? ` — ${supervision.state.migration.safe_failure}`
								: ""}
						</div>
					)}
					{supervision.state.shutdown && (
						<div className="flex items-center gap-2">
							<span>
								Application shutdown: {supervision.state.shutdown.phase}
							</span>
							{supervision.state.shutdown.actions.includes("retry_quit") && (
								<button
									type="button"
									className="rounded border border-border px-1.5 py-0.5"
									onClick={() => void supervision.retryQuit()}
								>
									Retry quit
								</button>
							)}
						</div>
					)}
					{supervision.state.shutdownOutcomeUnknown && (
						<div data-testid="shutdown-outcome-unknown">
							Application shutdown outcome unknown:{" "}
							{supervision.state.shutdownOutcomeUnknown.operation_id} —{" "}
							{supervision.state.shutdownOutcomeUnknown.intent.type} (
							{supervision.state.shutdownOutcomeUnknown.intent.code})
						</div>
					)}
					{supervision.state.shutdownTargets
						.filter((target) => target.actions.includes("retry_same_effect"))
						.map((target) => (
							<div key={target.ordinal} className="flex items-center gap-2">
								<span>
									Shutdown target {target.kind} requires reconciliation
								</span>
								<button
									type="button"
									className="rounded border border-border px-1.5 py-0.5"
									onClick={() => void supervision.retryShutdownTarget(target)}
								>
									Retry same effect
								</button>
							</div>
						))}
					{supervision.state.recovery.map((entry) => (
						<div key={entry.obligation_id} className="flex items-center gap-2">
							<span>{entry.safe_label}</span>
							{entry.actions.map((action) => (
								<button
									type="button"
									key={action}
									className="rounded border border-border px-1.5 py-0.5"
									onClick={() =>
										void supervision.requestRecovery(entry, action)
									}
								>
									{action.replace(/_/g, " ")}
								</button>
							))}
						</div>
					))}
				</div>
			)}
			{isFileDragOver && (
				<div className="absolute inset-0 flex items-center justify-center bg-primary/10 border-2 border-dashed border-primary rounded pointer-events-none z-10">
					<span className="text-sm font-medium text-primary bg-background/80 px-3 py-1.5 rounded">
						Drop image to attach
					</span>
				</div>
			)}
			<div
				ref={setScrollElement}
				onScroll={handleScroll}
				data-testid="chat-session-scroll"
				className="flex-1 min-h-0 overflow-auto select-text"
			>
				<div
					className="relative py-2"
					style={{ height: messageVirtualizer.getTotalSize() }}
				>
					{messageVirtualizer.getVirtualItems().map((virtualItem) => {
						const idx = virtualItem.index;
						if (idx >= session.messages.length) {
							return (
								<div
									key={virtualItem.key}
									data-index={virtualItem.index}
									ref={messageVirtualizer.measureElement}
									className="absolute left-0 top-0 w-full"
									style={{
										transform: `translateY(${virtualItem.start}px)`,
									}}
								>
									<ShimmerPlaceholder lines={shimmerLineCount} />
								</div>
							);
						}

						const msg = session.messages[idx];
						if (!msg) return null;
						if (msg.role !== "agent") {
							const textContent = getTextContent(msg.parts);
							const imageParts = msg.parts.filter(
								(p): p is DisplayImagePart =>
									p.type === "image" || p.type === "image_ref",
							);
							const isActiveSearchMessage =
								activeSearchMatch?.messageId === msg.id;
							return (
								<div
									key={virtualItem.key}
									data-index={virtualItem.index}
									ref={(node) => {
										messageVirtualizer.measureElement(node);
										if (node) {
											messageRefs.current.set(msg.id, node);
										} else {
											messageRefs.current.delete(msg.id);
										}
									}}
									className={
										isActiveSearchMessage
											? "group absolute left-0 top-0 w-full rounded-sm bg-primary/10 ring-1 ring-primary/30"
											: "group absolute left-0 top-0 w-full"
									}
									style={{
										transform: `translateY(${virtualItem.start}px)`,
									}}
								>
									<StreamMessage
										content={textContent}
										role={msg.role}
										images={imageParts.length > 0 ? imageParts : undefined}
										mentions={msg.mentions}
										timestamp={msg.timestamp}
										sessionId={session.id}
									/>
								</div>
							);
						}

						const isLastMsg = idx === session.messages.length - 1;
						const isLastAgentStreaming = isStreaming && isLastMsg;
						const isActiveSearchMessage =
							activeSearchMatch?.messageId === msg.id;

						return (
							<div
								key={virtualItem.key}
								data-index={virtualItem.index}
								ref={(node) => {
									messageVirtualizer.measureElement(node);
									if (node) {
										messageRefs.current.set(msg.id, node);
									} else {
										messageRefs.current.delete(msg.id);
									}
								}}
								className={
									isActiveSearchMessage
										? "group absolute left-0 top-0 w-full rounded-sm bg-primary/10 ring-1 ring-primary/30"
										: "group absolute left-0 top-0 w-full"
								}
								style={{
									transform: `translateY(${virtualItem.start}px)`,
								}}
							>
								<AgentMessageParts
									msg={msg}
									sessionId={session.id}
									isLastAgentStreaming={isLastAgentStreaming}
									worktreePath={worktreePath}
									showThinkingContent={showThinkingContent}
									rawScrollback={rawScrollback}
									onOpenDiffFile={onOpenDiffFile}
									permissionVisibilityRoot={permissionVisibilityRoot}
									respondPermission={onRespondPermission}
								/>
								<AgentMessageMeta
									msg={msg}
									isStreaming={isLastAgentStreaming}
									interruption={
										session.lastTurnInterruption?.messageId === msg.id
											? session.lastTurnInterruption
											: null
									}
								/>
							</div>
						);
					})}
				</div>
				{fallbackPendingPermission && (
					<PermissionDialogSlot
						request={fallbackPendingPermission}
						status="pending"
						sessionId={session.id}
						worktreePath={worktreePath}
						onOpenDiffFile={onOpenDiffFile}
						visibilityRoot={permissionVisibilityRoot}
						respondPermission={onRespondPermission}
					/>
				)}
			</div>
			<div className="shrink-0">
				{nativeCommandNotice && (
					<div className="px-3 pb-2">
						<div
							className={
								nativeCommandNotice.kind === "error"
									? "rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
									: "rounded border border-border bg-muted/40 px-3 py-2 text-xs"
							}
							role={nativeCommandNotice.kind === "error" ? "alert" : "status"}
						>
							<div className="flex items-start justify-between gap-2">
								<div className="min-w-0">
									<div className="font-medium">{nativeCommandNotice.title}</div>
									{nativeCommandNotice.detail && (
										<div className="mt-1 whitespace-pre-wrap break-words text-muted-foreground">
											{nativeCommandNotice.detail}
										</div>
									)}
									{nativeCommandNotice.taskItems &&
										nativeCommandNotice.taskItems.length > 0 && (
											<div className="mt-2 space-y-1">
												{nativeCommandNotice.taskItems.map((item) => {
													const isDone =
														item.status === "completed" ||
														item.status === "failed" ||
														item.status === "stopped";
													return (
														<div
															key={item.toolUseId}
															className="flex min-w-0 items-center gap-2 rounded border border-border/60 bg-background/70 px-2 py-1"
														>
															<span
																className={
																	isDone
																		? "size-2 shrink-0 rounded-full bg-muted-foreground/50"
																		: "size-2 shrink-0 rounded-full bg-primary"
																}
																aria-hidden="true"
															/>
															<span className="shrink-0 text-[11px] uppercase tracking-normal text-muted-foreground">
																{item.status}
																{item.background ? " background" : ""}
															</span>
															<span className="min-w-0 truncate text-foreground">
																{item.label}
															</span>
														</div>
													);
												})}
											</div>
										)}
								</div>
								<button
									type="button"
									className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
									aria-label="Dismiss command result"
									onClick={() => setNativeCommandNotice(null)}
								>
									<X className="size-3.5" />
								</button>
							</div>
						</div>
					</div>
				)}
				{isSearchOpen && (
					<div className="px-3 pb-2">
						<div className="flex items-center gap-1 rounded border border-border bg-background px-2 py-1">
							<Search className="size-3.5 shrink-0 text-muted-foreground" />
							<input
								ref={searchInputRef}
								type="search"
								value={searchQuery}
								onChange={(event) => {
									setSearchQuery(event.target.value);
									setActiveSearchIndex(0);
								}}
								onKeyDown={(event) => {
									if (event.key === "Enter") {
										event.preventDefault();
										if (event.shiftKey) {
											goToPreviousSearchMatch();
										} else {
											goToNextSearchMatch();
										}
									} else if (event.key === "Escape") {
										closeThreadSearch();
									}
								}}
								aria-label="Find in current thread"
								placeholder="Find in current thread"
								className="min-w-0 flex-1 bg-transparent text-sm outline-none"
							/>
							<span className="w-14 shrink-0 text-right text-xs text-muted-foreground tabular-nums">
								{searchQuery.trim()
									? `${searchMatches.length === 0 ? 0 : activeSearchIndex + 1}/${searchMatches.length}`
									: "0/0"}
							</span>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
								aria-label="Previous search match"
								onClick={goToPreviousSearchMatch}
								disabled={searchMatches.length === 0}
							>
								<ChevronUp className="size-3.5" />
							</button>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
								aria-label="Next search match"
								onClick={goToNextSearchMatch}
								disabled={searchMatches.length === 0}
							>
								<ChevronDown className="size-3.5" />
							</button>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
								aria-label="Close thread search"
								onClick={closeThreadSearch}
							>
								<X className="size-3.5" />
							</button>
						</div>
					</div>
				)}
				<div className="flex items-center gap-2 px-4 pb-1 text-xs text-muted-foreground">
					{activityStatus && (
						<div className="min-w-0 flex-1 animate-pulse truncate">
							{activityStatus.label}
						</div>
					)}
					{session.messages.length > 0 && (
						<button
							type="button"
							className={`${activityStatus ? "" : "ml-auto"} inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground`}
							aria-label="Find in current thread"
							title="Find in current thread"
							onClick={openThreadSearch}
						>
							<Search className="size-3.5" />
						</button>
					)}
					{session.messages.length > 0 && (
						<button
							type="button"
							className={`inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 hover:bg-muted hover:text-foreground ${
								rawScrollback
									? "bg-muted text-foreground"
									: "text-muted-foreground"
							}`}
							aria-label={
								rawScrollback
									? "Disable raw scrollback"
									: "Enable raw scrollback"
							}
							title="Toggle raw scrollback"
							onClick={toggleRawScrollback}
						>
							<Code2 className="size-3.5" />
						</button>
					)}
					{hasThinkingContent && (
						<button
							type="button"
							className="inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
							aria-label={
								showThinkingContent ? "Hide thinking" : "Show thinking"
							}
							title="Toggle thinking visibility"
							onClick={() => setShowThinkingContent((current) => !current)}
						>
							<ChevronRight
								className={`size-3.5 transition-transform ${showThinkingContent ? "rotate-90" : ""}`}
							/>
						</button>
					)}
					{latestCopyableAgentText && (
						<button
							type="button"
							className="inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
							aria-label="Copy latest agent response"
							title="Copy latest agent response"
							onClick={handleCopyLatestAgentText}
							disabled={copyState === "copied"}
						>
							{copyState === "copied" ? (
								<Check className="size-3.5" />
							) : (
								<Copy className="size-3.5" />
							)}
							<span className="sr-only">
								{copyState === "copied"
									? "Copied"
									: copyState === "error"
										? "Copy failed"
										: "Copy"}
							</span>
						</button>
					)}
				</div>
				{error && (
					<div className="px-2 pb-2">
						<div
							className="flex items-start gap-2 rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive"
							role="alert"
						>
							<span className="min-w-0 flex-1">{error}</span>
							<button
								type="button"
								className="shrink-0 rounded p-0.5 hover:bg-destructive/10"
								aria-label="Dismiss error"
								onClick={onDismissError}
							>
								<X className="size-3.5" />
							</button>
						</div>
					</div>
				)}
				<SessionFeedbackBanners
					entries={feedback}
					onDismiss={onDismissFeedback}
					onRetry={onRetryFeedback}
					hasMore={hasMoreFeedback}
					onLoadMore={onLoadMoreFeedback}
				/>
				{feedback.length === 0 && notice && (
					<div className="px-2 pb-2">
						<div
							className={
								notice.kind === "persist_failure"
									? "flex items-start gap-2 rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
									: "flex items-start gap-2 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
							}
							role={notice.kind === "persist_failure" ? "alert" : "status"}
							data-testid="session-notice-banner"
						>
							<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
							<span>{notice.message}</span>
						</div>
					</div>
				)}
				{session.contextCarry === "failed" && (
					<div className="px-2 pb-2">
						<div
							className="flex items-start gap-2 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
							role="status"
						>
							<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
							<span>
								Conversation context was not restored. New replies will continue
								without prior agent memory.
							</span>
						</div>
					</div>
				)}
				{isStreaming && stallObservation && stallIdleLabel && (
					<div className="px-2 pb-2">
						<div
							className="flex items-start gap-2 rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
							role="status"
						>
							<AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
							<span>
								No agent output for {stallIdleLabel}. Session remains active.
							</span>
						</div>
					</div>
				)}
				{pendingQueue.length > 0 && (
					<div className="px-3 pb-2 space-y-1">
						{queuePaused && (
							<div className="flex items-center justify-between rounded border border-amber-500/40 bg-amber-500/10 px-2 py-1.5 text-xs">
								<span className="text-amber-700 dark:text-amber-300">
									Queue paused
								</span>
								<button
									type="button"
									className="rounded px-2 py-1 font-medium hover:bg-amber-500/15"
									onClick={() => void onResumeQueue()}
								>
									Resume queue
								</button>
							</div>
						)}
						{pendingQueue.map((turn, index) => (
							<div
								key={turn.id}
								className="flex items-center gap-2 rounded border border-border bg-muted/40 px-2 py-1.5 text-xs"
							>
								<span className="shrink-0 text-muted-foreground">
									Queued {index + 1}
								</span>
								<span className="min-w-0 flex-1 truncate">
									{turn.contentPreview || "[image]"}
								</span>
								<button
									type="button"
									className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
									aria-label="Cancel queued message"
									onClick={() => onCancelQueuedTurn(turn.id)}
								>
									<X className="size-3.5" />
								</button>
							</div>
						))}
					</div>
				)}
				<TodoListFooter snapshot={todoListSnapshot} />
				<MessageInput
					ref={messageInputRef}
					onSend={handleComposerSend}
					onInterrupt={onInterrupt}
					isStreaming={isStreaming}
					isInterrupting={isInterrupting}
					onCycleMode={cycleMode}
					mode={permissionMode}
					onModeChange={handlePermissionModeChange}
					planMode={planMode}
					onPlanModeChange={onPlanModeChange}
					models={availableModels}
					backends={backends}
					currentModelId={selectedModel}
					onModelChange={onModelChange}
					currentBackendId={selectedBackendId}
					canChangeBackend={canChangeBackend}
					worktreePath={worktreePath}
					promptSuggestion={promptSuggestion}
					runtimeSlashCommands={runtimeSlashCommands}
				/>
			</div>
		</div>
	);
}
