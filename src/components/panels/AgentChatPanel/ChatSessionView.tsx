import { useVirtualizer } from "@tanstack/react-virtual";
import { invoke } from "@tauri-apps/api/core";
import {
	Check,
	ChevronDown,
	ChevronRight,
	ChevronUp,
	Code2,
	Copy,
	Download,
	FilePlus2,
	FileText,
	Gauge,
	History,
	Minimize2,
	MoreHorizontal,
	Radio,
	RotateCcw,
	Search,
	Stethoscope,
	Target,
	Terminal,
	Wrench,
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
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { DropZoneType } from "@/hooks/useNativeFileDrop";
import { useOneShotPty } from "@/hooks/useOneShotPty";
import { getRewindWorktreeCheckpointPreview } from "@/hooks/useSessionStore";
import type {
	AgentEditorContext,
	AgentEditorSelection,
	BackendInfo,
	ChatMessage,
	ChatSession,
	CodexGoal,
	CodexRuntimeStatus,
	ImageAttachment,
	ImagePart,
	MentionReference,
	MessagePart,
	ModelInfo,
	PermissionMode,
	QueuedAgentTurn,
	SessionStatus,
	SlashCommand,
	TokenUsage,
} from "@/types/session";
import { getTextContent, PERMISSION_MODE_LABELS } from "@/types/session";
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

interface ThreadSearchMatch {
	messageId: string;
	matchIndex: number;
}

interface NativeCommandNotice {
	kind: "info" | "error";
	title: string;
	detail?: string;
	permissionProfiles?: AgentCodexPermissionProfile[];
	taskItems?: AgentTaskListReport["items"];
	showGoalEditor?: boolean;
	codexGoal?: CodexGoal | null;
	exportTranscript?: {
		content: string;
	};
	copyOptions?: Array<{
		id: string;
		label: string;
		content: string;
		suggestedPath?: string | null;
	}>;
}

interface AgentCodexPermissionProfile {
	id: string;
	description?: string | null;
}

interface AgentCopyWriteResult {
	title: string;
	detail: string;
	path: string;
	byteCount: number;
}

interface AgentCopyResponse {
	title: string;
	detail: string;
	content: string;
	ordinal: number;
	messageId: string;
	suggestedPath: string;
	codeBlocks: Array<{
		index: number;
		language?: string | null;
		label: string;
		content: string;
		lineCount: number;
		suggestedPath: string;
	}>;
}

interface AgentContextCompactResult {
	title: string;
	detail: string;
}

interface AgentBackgroundTerminalCleanResult {
	title: string;
	detail: string;
}

interface AgentCodexShellCommandResult {
	title: string;
	detail: string;
	command: string;
}

interface AgentCodexRealtimeResult {
	title: string;
	detail: string;
}

interface AgentReviewStartResult {
	title: string;
	detail: string;
}

interface AgentCodexAccountStatusResult {
	title: string;
	detail: string;
}

interface AgentCodexRuntimeInventoryResult {
	title: string;
	detail: string;
}

interface AgentCodexGoalResult {
	title: string;
	detail: string;
	goal?: CodexGoal | null;
}

interface AgentExportTranscriptResult {
	title: string;
	detail: string;
	content: string;
	path?: string | null;
	suggestedPath?: string | null;
	messageCount: number;
}

interface AgentInitScaffoldResult {
	path: string;
	created: boolean;
	content: string;
}

interface AgentDoctorReport {
	title: string;
	detail: string;
	okCount: number;
	warningCount: number;
	errorCount: number;
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

interface AgentShellCommandResult {
	title: string;
	detail: string;
	prompt: string;
	exitCode?: number | null;
	timedOut: boolean;
}

interface AgentShellBackgroundOutput {
	output: string;
	truncated: boolean;
	path: string;
}

interface AgentPreparedShellCommand {
	command: string;
	displayCommand: string;
	label: string;
	timeoutSecs?: number | null;
	background: boolean;
	backgroundOutputPath?: string;
}

interface AgentPromptSuggestion {
	text: string;
	source: string;
}

interface ActiveShellCommand {
	ptyId: number;
	command: string;
	background: boolean;
	backgroundOutputPath?: string;
	contextSent: boolean;
}

interface QueuedShellCommand {
	id: string;
	prepared: AgentPreparedShellCommand;
}

function RewindMessageButton({ onClick }: { onClick: () => void }) {
	return (
		<button
			type="button"
			className="absolute top-1 right-2 z-10 inline-flex size-6 items-center justify-center rounded bg-background/90 text-muted-foreground opacity-0 shadow-sm ring-1 ring-border/70 transition-opacity hover:bg-muted hover:text-foreground focus:opacity-100 group-hover:opacity-100"
			aria-label="Rewind to this message"
			title="Rewind to this message"
			onClick={onClick}
		>
			<RotateCcw className="size-3.5" />
		</button>
	);
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
	const trimmed = content.trim();
	if (!trimmed) return null;

	return (
		<div className="px-5 py-0.5 text-xs" data-testid="thinking-block">
			<button
				type="button"
				className="flex min-w-0 items-center gap-1 text-muted-foreground/70 hover:text-foreground/80"
				onClick={() => setIsExpanded((current) => !current)}
				aria-expanded={isExpanded}
			>
				<ChevronRight
					className={`size-3 shrink-0 transition-transform ${isExpanded ? "rotate-90" : ""}`}
				/>
				<span className={isStreaming ? "animate-pulse" : ""}>Thinking</span>
			</button>
			{showContent && isExpanded && (
				<div className="mt-1 ml-4 max-h-48 overflow-y-auto whitespace-pre-wrap break-words rounded border border-border/60 bg-muted/30 px-2 py-1.5 text-muted-foreground">
					{trimmed}
				</div>
			)}
		</div>
	);
}

interface AgentMessagePartsProps {
	msg: ChatMessage;
	isLastAgentStreaming: boolean;
	worktreePath: string;
	showThinkingContent: boolean;
	rawScrollback: boolean;
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
	showThinkingContent,
	rawScrollback,
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
								worktreePath={worktreePath}
								onAllow={(id, updatedInput) =>
									respondPermission(id, true, updatedInput)
								}
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
	sessionStatus: SessionStatus | null;
	isStreaming: boolean;
	activityStatus: { label: string } | null;
	error: string | null;
	permissionMode: PermissionMode;
	availableModels: ModelInfo[];
	selectedModel: string;
	pendingQueue: QueuedAgentTurn[];
	latestTokenUsage: TokenUsage | null;
	codexGoal?: CodexGoal | null;
	codexRuntimeStatus?: CodexRuntimeStatus | null;
	runtimeSlashCommands?: SlashCommand[];
	backends: BackendInfo[];
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
	) => Promise<void>;
	onInterrupt: () => void;
	onCancelQueuedTurn: (queuedTurnId?: string | null) => Promise<void>;
	onRewindToMessage?: (
		messageId: string,
		options?: { restoreWorktree?: boolean },
	) => Promise<void>;
	onPermissionModeChange: (mode: PermissionMode) => void;
	onModelChange: (modelId: string) => void;
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
	sessionStatus,
	isStreaming,
	activityStatus,
	error,
	permissionMode,
	availableModels,
	selectedModel,
	pendingQueue,
	latestTokenUsage,
	codexGoal = null,
	codexRuntimeStatus = null,
	runtimeSlashCommands = [],
	backends,
	selectedBackendId,
	canChangeBackend,
	worktreePath,
	activeEditorPath,
	openEditorPaths,
	activeEditorSelection,
	onSend,
	onInterrupt,
	onCancelQueuedTurn,
	onRewindToMessage,
	onPermissionModeChange,
	onModelChange,
	onBackendChange,
	onRespondPermission,
	registerDropZone,
	dropZoneName,
	sendMessageRef,
}: ChatSessionViewProps) {
	const messageInputRef = useRef<MessageInputHandle>(null);
	const copyWritePathInputRef = useRef<HTMLInputElement>(null);
	const exportWritePathInputRef = useRef<HTMLInputElement>(null);
	const [isFileDragOver, setIsFileDragOver] = useState(false);
	const [copyState, setCopyState] = useState<"idle" | "copied" | "error">(
		"idle",
	);
	const [promptSuggestion, setPromptSuggestion] = useState<string | null>(null);
	const [activeShellCommand, setActiveShellCommand] =
		useState<ActiveShellCommand | null>(null);
	const [queuedShellCommands, setQueuedShellCommands] = useState<
		QueuedShellCommand[]
	>([]);
	const queuedShellCommandIdRef = useRef(0);
	const shellContextSentRef = useRef<Set<number>>(new Set());
	const {
		activePtys: shellPtys,
		spawn: spawnShellPty,
		cancel: cancelShellPty,
		getOutput: getShellOutput,
	} = useOneShotPty();
	const [threadSearchState, setThreadSearchState] = useState({
		open: false,
		requestId: 0,
	});
	const [searchQuery, setSearchQuery] = useState("");
	const [searchMatches, setSearchMatches] = useState<ThreadSearchMatch[]>([]);
	const [activeSearchIndex, setActiveSearchIndex] = useState(0);
	const [isStatusOpen, setIsStatusOpen] = useState(false);
	const [showThinkingContent, setShowThinkingContent] = useState(true);
	const [rawScrollback, setRawScrollback] = useState(false);
	const [nativeCommandNotice, setNativeCommandNotice] =
		useState<NativeCommandNotice | null>(null);
	const [selectedPermissionProfileId, setSelectedPermissionProfileId] =
		useState<string | null>(session.permissionProfileId ?? null);

	useEffect(() => {
		setSelectedPermissionProfileId(session.permissionProfileId ?? null);
	}, [session.permissionProfileId]);
	const [copyWritePath, setCopyWritePath] = useState("");
	const [copyWritePathEdited, setCopyWritePathEdited] = useState(false);
	const [exportWritePath, setExportWritePath] = useState("");
	const [goalDraft, setGoalDraft] = useState("");
	const [goalTokenBudgetDraft, setGoalTokenBudgetDraft] = useState("");
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
	const lastMessageCount = useRef(0);
	const isNearBottomRef = useRef(true);
	const lastTaskListRevisionRef = useRef<string | null>(null);
	const currentTaskListRevision = useMemo(
		() => taskListRevision(session.messages),
		[session.messages],
	);

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

	// Track scroll position via onScroll handler
	const handleScroll = useCallback(() => {
		const el = scrollRef.current;
		if (!el) return;
		isNearBottomRef.current =
			el.scrollHeight - el.scrollTop - el.clientHeight < 100;
	}, []);

	const handleRewindToMessage = useCallback(
		async (messageId: string) => {
			if (!onRewindToMessage) return;
			let restoreWorktree = false;
			try {
				const preview = await getRewindWorktreeCheckpointPreview(
					session.id,
					messageId,
				);
				const hasWorktreeEffect =
					preview.targetDirtyFileCount > 0 || preview.currentDirtyFileCount > 0;
				if (preview.available && hasWorktreeEffect) {
					restoreWorktree = window.confirm(
						[
							"Restore the worktree to this message checkpoint?",
							"",
							`Checkpoint changes: ${preview.targetDirtyFileCount}`,
							`Current changes: ${preview.currentDirtyFileCount}`,
							"",
							"Cancel keeps the current files and rewinds only the transcript.",
						].join("\n"),
					);
				}
			} catch {
				restoreWorktree = false;
			}
			await onRewindToMessage(
				messageId,
				restoreWorktree ? { restoreWorktree: true } : undefined,
			);
		},
		[onRewindToMessage, session.id],
	);

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
			if (index >= session.messages.length) return 56;
			return session.messages[index]?.role === "agent" ? 112 : 72;
		},
		overscan: 8,
		initialRect: {
			width: 800,
			height: 800,
		},
	});

	// Auto-scroll: keep tail-following behavior while only mounted rows are in the DOM.
	// biome-ignore lint/correctness/useExhaustiveDependencies: lastAgentContent/lastAgentPartsLen/shimmerLineCount intentionally trigger tail follow during streaming growth.
	useEffect(() => {
		const count = session.messages.length;
		if (count > lastMessageCount.current) {
			messageVirtualizer.scrollToIndex(Math.max(virtualRowCount - 1, 0), {
				align: "end",
			});
		} else if (isNearBottomRef.current) {
			messageVirtualizer.scrollToIndex(Math.max(virtualRowCount - 1, 0), {
				align: "end",
			});
		}
		lastMessageCount.current = count;
	}, [
		session.messages.length,
		lastAgentContent,
		lastAgentPartsLen,
		shimmerLineCount,
		virtualRowCount,
		messageVirtualizer,
	]);

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

	const activeShellInfo = activeShellCommand
		? (shellPtys.get(activeShellCommand.ptyId) ?? null)
		: null;
	const activeShellOutput = activeShellCommand
		? getShellOutput(activeShellCommand.ptyId)
		: "";

	useEffect(() => {
		if (!activeShellCommand || !activeShellInfo) return;
		const terminalStatus = [
			"completed",
			"error",
			"timeout",
			"cancelled",
		].includes(activeShellInfo.status);
		if (
			!terminalStatus ||
			activeShellCommand.contextSent ||
			shellContextSentRef.current.has(activeShellCommand.ptyId)
		)
			return;

		if (activeShellInfo.status === "cancelled") {
			setNativeCommandNotice({
				kind: "error",
				title: "Shell: cancelled",
				detail: activeShellCommand.command,
			});
			setActiveShellCommand(null);
			return;
		}

		shellContextSentRef.current.add(activeShellCommand.ptyId);
		let cancelled = false;
		void (async () => {
			let output = activeShellOutput;
			let truncated = false;
			if (activeShellCommand.backgroundOutputPath) {
				const backgroundOutput = await invoke<AgentShellBackgroundOutput>(
					"read_agent_shell_background_output",
					{
						outputPath: activeShellCommand.backgroundOutputPath,
					},
				);
				const backgroundBlock = backgroundOutput.output.trim()
					? `\n\nBackground output snapshot (${backgroundOutput.path}):\n${backgroundOutput.output}`
					: `\n\nBackground output snapshot (${backgroundOutput.path}):\n<empty>`;
				output = `${activeShellOutput}${backgroundBlock}`;
				truncated = backgroundOutput.truncated;
			}
			const result = await invoke<AgentShellCommandResult>(
				"build_agent_shell_command_context_prompt",
				{
					command: activeShellCommand.command,
					output,
					exitCode: activeShellInfo.exit_code,
					timedOut: activeShellInfo.status === "timeout",
					truncated,
				},
			);
			return result;
		})()
			.then(async (result) => {
				if (cancelled) return;
				setNativeCommandNotice({
					kind: result.timedOut || result.exitCode ? "error" : "info",
					title: result.title,
					detail: result.detail,
				});
				await onSend(result.prompt, undefined, undefined, {
					editorContext: currentEditorContext,
				});
				if (!cancelled) {
					setActiveShellCommand(null);
				}
			})
			.catch((e) => {
				if (cancelled) return;
				setNativeCommandNotice({
					kind: "error",
					title: "Shell command failed",
					detail: String(e),
				});
				setActiveShellCommand(null);
			});
		return () => {
			cancelled = true;
		};
	}, [
		activeShellCommand,
		activeShellInfo,
		activeShellOutput,
		currentEditorContext,
		onSend,
	]);

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

	const prepareCopyResponse = useCallback(async () => {
		try {
			const result = await invoke<AgentCopyResponse>(
				"build_agent_copy_response",
				{
					chatSessionId: session.id,
					raw: undefined,
					excludeMessageId: undefined,
				},
			);
			setCopyWritePath(result.suggestedPath);
			setCopyWritePathEdited(false);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail:
					result.codeBlocks.length > 0
						? `${result.detail}\nChoose the whole response or a code block.`
						: `${result.detail}\nChoose whether to copy or write the whole response.`,
				copyOptions: [
					{
						id: `response-${result.messageId}`,
						label: "Whole response",
						content: result.content,
						suggestedPath: result.suggestedPath,
					},
					...result.codeBlocks.map((block) => ({
						id: `response-${result.messageId}-block-${block.index}`,
						label: block.label,
						content: block.content,
						suggestedPath: block.suggestedPath,
					})),
				],
			});
			requestAnimationFrame(() => {
				copyWritePathInputRef.current?.focus();
				copyWritePathInputRef.current?.select();
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Copy response failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const prepareTranscriptExport = useCallback(async () => {
		try {
			const result = await invoke<AgentExportTranscriptResult>(
				"build_agent_export_transcript",
				{
					chatSessionId: session.id,
					raw: undefined,
				},
			);
			setExportWritePath(result.suggestedPath ?? "");
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.path
					? result.detail
					: `${result.detail}\nChoose whether to copy the transcript or write it to a file.`,
				exportTranscript: result.path
					? undefined
					: {
							content: result.content,
						},
			});
			if (!result.path) {
				requestAnimationFrame(() => {
					exportWritePathInputRef.current?.focus();
					exportWritePathInputRef.current?.select();
				});
			}
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Export failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const createAgentsGuidance = useCallback(async () => {
		try {
			const result = await invoke<AgentInitScaffoldResult>(
				"create_agents_md_scaffold",
				{ worktreePath },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.created ? "AGENTS.md created" : "AGENTS.md exists",
				detail: result.created
					? `Created starter guidance at ${result.path}`
					: `Existing guidance left unchanged at ${result.path}`,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "AGENTS.md init failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const compactRuntimeContext = useCallback(async () => {
		try {
			const result = await invoke<AgentContextCompactResult>(
				"compact_agent_context",
				{
					chatSessionId: session.id,
				},
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Context compaction failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const cleanCodexBackgroundTerminals = useCallback(async () => {
		try {
			const result = await invoke<AgentBackgroundTerminalCleanResult>(
				"clean_codex_background_terminals",
				{
					chatSessionId: session.id,
				},
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex background cleanup failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const runCodexShellCommand = useCallback(async () => {
		try {
			const content = messageInputRef.current?.getDraft() ?? "";
			const result = await invoke<AgentCodexShellCommandResult>(
				"run_codex_shell_command",
				{
					chatSessionId: session.id,
					content,
				},
			);
			messageInputRef.current?.clearDraft();
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex shell command failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const startCodexRealtimeText = useCallback(async () => {
		try {
			const content = messageInputRef.current?.getDraft() ?? "";
			const result = await invoke<AgentCodexRealtimeResult>(
				"start_codex_realtime_text_session",
				{
					chatSessionId: session.id,
					content,
				},
			);
			if (content.trim()) {
				messageInputRef.current?.clearDraft();
			}
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex realtime start failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const appendCodexRealtimeText = useCallback(async () => {
		try {
			const content = messageInputRef.current?.getDraft() ?? "";
			const result = await invoke<AgentCodexRealtimeResult>(
				"append_codex_realtime_text",
				{
					chatSessionId: session.id,
					content,
				},
			);
			messageInputRef.current?.clearDraft();
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex realtime append failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const stopCodexRealtime = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRealtimeResult>(
				"stop_codex_realtime_session",
				{
					chatSessionId: session.id,
				},
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex realtime stop failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const startCodexReview = useCallback(
		async (targetType = "uncommittedChanges", targetValue?: string) => {
			try {
				const result = await invoke<AgentReviewStartResult>(
					"start_codex_uncommitted_changes_review",
					{ chatSessionId: session.id, targetType, targetValue },
				);
				setNativeCommandNotice({
					kind: "info",
					title: result.title,
					detail: result.detail,
				});
			} catch (e) {
				setNativeCommandNotice({
					kind: "error",
					title: "Codex review failed",
					detail: String(e),
				});
			}
		},
		[session.id],
	);

	const startPromptedCodexReview = useCallback(
		async (targetType: "baseBranch" | "commit" | "custom") => {
			const promptLabel =
				targetType === "baseBranch"
					? "Base branch"
					: targetType === "commit"
						? "Commit SHA"
						: "Review instructions";
			const value = window.prompt(promptLabel);
			if (!value?.trim()) return;
			await startCodexReview(targetType, value.trim());
		},
		[startCodexReview],
	);

	const showCodexGoal = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexGoalResult>(
				"read_codex_thread_goal",
				{ chatSessionId: session.id },
			);
			setGoalDraft(result.goal?.objective ?? "");
			setGoalTokenBudgetDraft(
				result.goal?.tokenBudget ? String(result.goal.tokenBudget) : "",
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
				showGoalEditor: true,
				codexGoal: result.goal ?? null,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex goal failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const saveCodexGoal = useCallback(
		async (status?: string) => {
			const objective = goalDraft.trim();
			const tokenBudgetText = goalTokenBudgetDraft.trim();
			let tokenBudget: number | undefined;
			if (!objective) {
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								kind: "error",
								detail: "Enter a goal objective before saving.",
							}
						: current,
				);
				return;
			}
			if (tokenBudgetText) {
				const parsedTokenBudget = Number.parseInt(tokenBudgetText, 10);
				if (!Number.isFinite(parsedTokenBudget) || parsedTokenBudget <= 0) {
					setNativeCommandNotice((current) =>
						current
							? {
									...current,
									kind: "error",
									detail: "Token budget must be a positive number.",
								}
							: current,
					);
					return;
				}
				tokenBudget = parsedTokenBudget;
			}
			try {
				const result = await invoke<AgentCodexGoalResult>(
					"set_codex_thread_goal",
					{
						chatSessionId: session.id,
						objective,
						status: status ?? undefined,
						tokenBudget,
					},
				);
				setGoalDraft(result.goal?.objective ?? objective);
				setGoalTokenBudgetDraft(
					result.goal?.tokenBudget ? String(result.goal.tokenBudget) : "",
				);
				setNativeCommandNotice({
					kind: "info",
					title: result.title,
					detail: result.detail,
					showGoalEditor: true,
					codexGoal: result.goal ?? null,
				});
			} catch (e) {
				setNativeCommandNotice({
					kind: "error",
					title: "Codex goal update failed",
					detail: String(e),
					showGoalEditor: true,
					codexGoal: nativeCommandNotice?.codexGoal ?? null,
				});
			}
		},
		[
			goalDraft,
			goalTokenBudgetDraft,
			nativeCommandNotice?.codexGoal,
			session.id,
		],
	);

	const clearCodexGoal = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexGoalResult>(
				"clear_codex_thread_goal",
				{ chatSessionId: session.id },
			);
			setGoalDraft("");
			setGoalTokenBudgetDraft("");
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
				showGoalEditor: true,
				codexGoal: null,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex goal clear failed",
				detail: String(e),
				showGoalEditor: true,
				codexGoal: nativeCommandNotice?.codexGoal ?? null,
			});
		}
	}, [nativeCommandNotice?.codexGoal, session.id]);

	const updateCodexGoalStatus = useCallback(
		async (goal: CodexGoal, status: string) => {
			try {
				const result = await invoke<AgentCodexGoalResult>(
					"set_codex_thread_goal",
					{
						chatSessionId: session.id,
						objective: goal.objective,
						status,
						tokenBudget: goal.tokenBudget ?? undefined,
					},
				);
				setNativeCommandNotice({
					kind: "info",
					title: result.title,
					detail: result.detail,
					showGoalEditor: false,
					codexGoal: result.goal ?? null,
				});
			} catch (e) {
				setNativeCommandNotice({
					kind: "error",
					title: "Codex goal update failed",
					detail: String(e),
				});
			}
		},
		[session.id],
	);

	const clearCodexGoalFromRow = useCallback(async () => {
		await clearCodexGoal();
	}, [clearCodexGoal]);

	const showCodexAccountStatus = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexAccountStatusResult>(
				"read_codex_account_status",
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex account usage failed",
				detail: String(e),
			});
		}
	}, []);

	const showCodexThreadHistory = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_thread_history_report",
				{ worktreePath, query: searchQuery.trim() || null },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex thread history failed",
				detail: String(e),
			});
		}
	}, [searchQuery, worktreePath]);

	const showCodexThreadTranscript = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_thread_transcript_report",
				{ chatSessionId: session.id },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex thread transcript failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const showCodexHooksReport = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_hooks_report",
				{ worktreePath },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex hooks failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const showCodexRealtimeVoicesReport = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_realtime_voices_report",
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex realtime voices failed",
				detail: String(e),
			});
		}
	}, []);

	const applyCodexPermissionProfile = useCallback(
		async (permissionProfileId: string | null) => {
			if (!session.id) return;
			try {
				await invoke("set_codex_permission_profile", {
					chatSessionId: session.id,
					permissionProfileId,
				});
				setSelectedPermissionProfileId(permissionProfileId);
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								kind: "info",
								title: permissionProfileId
									? "Codex permission profile applied"
									: "Codex permission profile cleared",
							}
						: null,
				);
			} catch (e) {
				setNativeCommandNotice({
					kind: "error",
					title: "Codex permission profile failed",
					detail: String(e),
				});
			}
		},
		[session.id],
	);

	const showCodexPermissionProfiles = useCallback(async () => {
		try {
			const profiles = await invoke<AgentCodexPermissionProfile[]>(
				"read_codex_permission_profiles",
				{ worktreePath },
			);
			setNativeCommandNotice({
				kind: "info",
				title: `Codex permission profiles: ${profiles.length}`,
				permissionProfiles: profiles,
				detail:
					profiles.length === 0
						? "No permission profiles returned."
						: undefined,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex permission profiles failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const showCodexMcpStatusReport = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_mcp_status_report",
				{ chatSessionId: session.id },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex MCP status failed",
				detail: String(e),
			});
		}
	}, [session.id]);

	const showCodexRuntimeConfigReport = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_runtime_config_report",
				{ worktreePath },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex runtime config failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const showCodexRuntimeCapabilitiesReport = useCallback(async () => {
		try {
			const result = await invoke<AgentCodexRuntimeInventoryResult>(
				"read_codex_runtime_capabilities_report",
				{ chatSessionId: session.id, worktreePath },
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Codex runtime capabilities failed",
				detail: String(e),
			});
		}
	}, [session.id, worktreePath]);

	const showDebugConfigReport = useCallback(async () => {
		try {
			const report = await invoke<string>("build_agent_debug_config_report", {
				worktreePath,
			});
			setNativeCommandNotice({
				kind: "info",
				title: "Debug config",
				detail: report,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Debug config failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const showDoctorReport = useCallback(async () => {
		try {
			const report = await invoke<AgentDoctorReport>(
				"build_agent_doctor_report",
				{
					worktreePath,
				},
			);
			setNativeCommandNotice({
				kind: report.errorCount > 0 ? "error" : "info",
				title: report.title,
				detail: report.detail,
			});
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Doctor failed",
				detail: String(e),
			});
		}
	}, [worktreePath]);

	const copyNativeNoticeOption = useCallback(
		async (option: { label: string; content: string }) => {
			try {
				await navigator.clipboard.writeText(option.content);
				setCopyState("copied");
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								detail: `Copied ${option.label}.`,
							}
						: current,
				);
			} catch (e) {
				setCopyState("error");
				setNativeCommandNotice({
					kind: "error",
					title: "Copy failed",
					detail: String(e),
				});
			}
		},
		[],
	);

	const writeNativeNoticeOption = useCallback(
		async (option: {
			label: string;
			content: string;
			suggestedPath?: string | null;
		}) => {
			const rawPath = (
				copyWritePathEdited
					? copyWritePath
					: (option.suggestedPath ?? copyWritePath)
			).trim();
			if (!rawPath) {
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								kind: "error",
								detail: "Enter a relative file path before writing.",
							}
						: current,
				);
				return;
			}
			if (!copyWritePathEdited) {
				setCopyWritePath(rawPath);
			}
			try {
				const result = await invoke<AgentCopyWriteResult>(
					"write_agent_copy_selection_to_file",
					{
						worktreePath,
						rawPath,
						content: option.content,
					},
				);
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								kind: "info",
								title: result.title,
								detail: `${result.detail}\nSelection: ${option.label}`,
							}
						: {
								kind: "info",
								title: result.title,
								detail: `${result.detail}\nSelection: ${option.label}`,
							},
				);
				setCopyWritePath("");
				setCopyWritePathEdited(false);
			} catch (e) {
				setNativeCommandNotice((current) =>
					current
						? {
								...current,
								kind: "error",
								title: "Write failed",
								detail: String(e),
							}
						: {
								kind: "error",
								title: "Write failed",
								detail: String(e),
							},
				);
			}
		},
		[copyWritePath, copyWritePathEdited, worktreePath],
	);

	const copyExportTranscript = useCallback(async () => {
		const content = nativeCommandNotice?.exportTranscript?.content;
		if (!content) return;
		try {
			await navigator.clipboard.writeText(content);
			setCopyState("copied");
			setNativeCommandNotice((current) =>
				current
					? {
							...current,
							detail: "Copied transcript to clipboard.",
						}
					: current,
			);
		} catch (e) {
			setCopyState("error");
			setNativeCommandNotice({
				kind: "error",
				title: "Export copy failed",
				detail: String(e),
			});
		}
	}, [nativeCommandNotice?.exportTranscript?.content]);

	const writeExportTranscript = useCallback(async () => {
		const raw = exportWritePath.trim();
		if (!raw) {
			setNativeCommandNotice((current) =>
				current
					? {
							...current,
							kind: "error",
							detail: "Enter a relative file path before writing.",
						}
					: current,
			);
			return;
		}
		try {
			const result = await invoke<AgentExportTranscriptResult>(
				"build_agent_export_transcript",
				{
					chatSessionId: session.id,
					raw,
				},
			);
			setNativeCommandNotice({
				kind: "info",
				title: result.title,
				detail: result.detail,
			});
			setExportWritePath("");
		} catch (e) {
			setNativeCommandNotice({
				kind: "error",
				title: "Export failed",
				detail: String(e),
			});
		}
	}, [exportWritePath, session.id]);

	const startPreparedShellCommand = useCallback(
		async (prepared: AgentPreparedShellCommand) => {
			const info = await spawnShellPty(
				prepared.command,
				worktreePath,
				prepared.label,
				prepared.timeoutSecs ?? undefined,
			);
			setActiveShellCommand({
				ptyId: info.pty_id,
				command: prepared.displayCommand,
				background: prepared.background,
				backgroundOutputPath: prepared.backgroundOutputPath,
				contextSent: false,
			});
			setNativeCommandNotice(null);
		},
		[spawnShellPty, worktreePath],
	);

	useEffect(() => {
		if (isStreaming || activeShellCommand || queuedShellCommands.length === 0) {
			return;
		}
		const next = queuedShellCommands[0];
		setQueuedShellCommands((commands) => commands.slice(1));
		void startPreparedShellCommand(next.prepared).catch((e) => {
			setNativeCommandNotice({
				kind: "error",
				title: "Queued shell command failed",
				detail: String(e),
			});
		});
	}, [
		activeShellCommand,
		isStreaming,
		queuedShellCommands,
		startPreparedShellCommand,
	]);

	const handleComposerSend = useCallback(
		async (
			content: string,
			images?: ImageAttachment[],
			mentions?: MentionReference[],
		) => {
			if (!images || images.length === 0) {
				try {
					const prepared = await invoke<AgentPreparedShellCommand | null>(
						"prepare_agent_shell_input",
						{
							content,
						},
					);
					if (
						prepared &&
						typeof prepared === "object" &&
						"command" in prepared &&
						typeof prepared.command === "string"
					) {
						if (isStreaming) {
							queuedShellCommandIdRef.current += 1;
							setQueuedShellCommands((commands) => [
								...commands,
								{
									id: `shell-${queuedShellCommandIdRef.current}`,
									prepared,
								},
							]);
							setNativeCommandNotice({
								kind: "info",
								title: prepared.background
									? "Shell background queued"
									: "Shell queued",
								detail: prepared.displayCommand,
							});
							return;
						}
						await startPreparedShellCommand(prepared);
						return;
					}
				} catch (e) {
					setNativeCommandNotice({
						kind: "error",
						title: "Shell command failed",
						detail: String(e),
					});
					return;
				}
			}
			await onSend(content, images, mentions, {
				editorContext: currentEditorContext,
			});
		},
		[isStreaming, onSend, currentEditorContext, startPreparedShellCommand],
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
		const handleCopyEvent = () => {
			void handleCopyLatestAgentText();
		};
		window.addEventListener("agent-copy-latest-response", handleCopyEvent);
		return () =>
			window.removeEventListener("agent-copy-latest-response", handleCopyEvent);
	}, [handleCopyLatestAgentText]);

	useEffect(() => {
		const handleCopyOptionsEvent = () => {
			void prepareCopyResponse();
		};
		window.addEventListener(
			"agent-copy-response-options",
			handleCopyOptionsEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-copy-response-options",
				handleCopyOptionsEvent,
			);
		};
	}, [prepareCopyResponse]);

	useEffect(() => {
		const handleRawScrollbackEvent = () => {
			toggleRawScrollback();
		};
		window.addEventListener(
			"agent-toggle-raw-scrollback",
			handleRawScrollbackEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-toggle-raw-scrollback",
				handleRawScrollbackEvent,
			);
		};
	}, [toggleRawScrollback]);

	useEffect(() => {
		const handleExportTranscriptEvent = () => {
			void prepareTranscriptExport();
		};
		window.addEventListener(
			"agent-export-transcript",
			handleExportTranscriptEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-export-transcript",
				handleExportTranscriptEvent,
			);
		};
	}, [prepareTranscriptExport]);

	useEffect(() => {
		const handleCreateAgentsEvent = () => {
			void createAgentsGuidance();
		};
		window.addEventListener("agent-create-agents-md", handleCreateAgentsEvent);
		return () => {
			window.removeEventListener(
				"agent-create-agents-md",
				handleCreateAgentsEvent,
			);
		};
	}, [createAgentsGuidance]);

	useEffect(() => {
		const handleDebugConfigEvent = () => {
			void showDebugConfigReport();
		};
		window.addEventListener("agent-show-debug-config", handleDebugConfigEvent);
		return () => {
			window.removeEventListener(
				"agent-show-debug-config",
				handleDebugConfigEvent,
			);
		};
	}, [showDebugConfigReport]);

	useEffect(() => {
		const handleDoctorEvent = () => {
			void showDoctorReport();
		};
		window.addEventListener("agent-show-doctor", handleDoctorEvent);
		return () => {
			window.removeEventListener("agent-show-doctor", handleDoctorEvent);
		};
	}, [showDoctorReport]);

	useEffect(() => {
		const handleCodexAccountEvent = () => {
			void showCodexAccountStatus();
		};
		window.addEventListener(
			"agent-show-codex-account-usage",
			handleCodexAccountEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-account-usage",
				handleCodexAccountEvent,
			);
		};
	}, [showCodexAccountStatus]);

	useEffect(() => {
		const handleCodexGoalEvent = () => {
			void showCodexGoal();
		};
		window.addEventListener("agent-show-codex-goal", handleCodexGoalEvent);
		return () => {
			window.removeEventListener("agent-show-codex-goal", handleCodexGoalEvent);
		};
	}, [showCodexGoal]);

	useEffect(() => {
		const handleCompactCodexContextEvent = () => {
			void compactRuntimeContext();
		};
		window.addEventListener(
			"agent-compact-codex-context",
			handleCompactCodexContextEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-compact-codex-context",
				handleCompactCodexContextEvent,
			);
		};
	}, [compactRuntimeContext]);

	useEffect(() => {
		const handleCleanCodexBackgroundTerminalsEvent = () => {
			void cleanCodexBackgroundTerminals();
		};
		window.addEventListener(
			"agent-clean-codex-background-terminals",
			handleCleanCodexBackgroundTerminalsEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-clean-codex-background-terminals",
				handleCleanCodexBackgroundTerminalsEvent,
			);
		};
	}, [cleanCodexBackgroundTerminals]);

	useEffect(() => {
		const handleRunCodexShellCommandEvent = () => {
			void runCodexShellCommand();
		};
		window.addEventListener(
			"agent-run-codex-shell-command",
			handleRunCodexShellCommandEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-run-codex-shell-command",
				handleRunCodexShellCommandEvent,
			);
		};
	}, [runCodexShellCommand]);

	useEffect(() => {
		const handleStartCodexRealtimeTextEvent = () => {
			void startCodexRealtimeText();
		};
		window.addEventListener(
			"agent-start-codex-realtime-text",
			handleStartCodexRealtimeTextEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-start-codex-realtime-text",
				handleStartCodexRealtimeTextEvent,
			);
		};
	}, [startCodexRealtimeText]);

	useEffect(() => {
		const handleAppendCodexRealtimeTextEvent = () => {
			void appendCodexRealtimeText();
		};
		window.addEventListener(
			"agent-append-codex-realtime-text",
			handleAppendCodexRealtimeTextEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-append-codex-realtime-text",
				handleAppendCodexRealtimeTextEvent,
			);
		};
	}, [appendCodexRealtimeText]);

	useEffect(() => {
		const handleStopCodexRealtimeEvent = () => {
			void stopCodexRealtime();
		};
		window.addEventListener(
			"agent-stop-codex-realtime",
			handleStopCodexRealtimeEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-stop-codex-realtime",
				handleStopCodexRealtimeEvent,
			);
		};
	}, [stopCodexRealtime]);

	useEffect(() => {
		const handleStartCodexReviewEvent = () => {
			void startCodexReview();
		};
		window.addEventListener(
			"agent-start-codex-review",
			handleStartCodexReviewEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-start-codex-review",
				handleStartCodexReviewEvent,
			);
		};
	}, [startCodexReview]);

	useEffect(() => {
		const handleStartCodexReviewBaseBranchEvent = () => {
			void startPromptedCodexReview("baseBranch");
		};
		window.addEventListener(
			"agent-start-codex-review-base-branch",
			handleStartCodexReviewBaseBranchEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-start-codex-review-base-branch",
				handleStartCodexReviewBaseBranchEvent,
			);
		};
	}, [startPromptedCodexReview]);

	useEffect(() => {
		const handleStartCodexReviewCommitEvent = () => {
			void startPromptedCodexReview("commit");
		};
		window.addEventListener(
			"agent-start-codex-review-commit",
			handleStartCodexReviewCommitEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-start-codex-review-commit",
				handleStartCodexReviewCommitEvent,
			);
		};
	}, [startPromptedCodexReview]);

	useEffect(() => {
		const handleStartCodexReviewCustomEvent = () => {
			void startPromptedCodexReview("custom");
		};
		window.addEventListener(
			"agent-start-codex-review-custom",
			handleStartCodexReviewCustomEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-start-codex-review-custom",
				handleStartCodexReviewCustomEvent,
			);
		};
	}, [startPromptedCodexReview]);

	useEffect(() => {
		const handleCodexThreadHistoryEvent = () => {
			void showCodexThreadHistory();
		};
		window.addEventListener(
			"agent-show-codex-thread-history",
			handleCodexThreadHistoryEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-thread-history",
				handleCodexThreadHistoryEvent,
			);
		};
	}, [showCodexThreadHistory]);

	useEffect(() => {
		const handleCodexThreadTranscriptEvent = () => {
			void showCodexThreadTranscript();
		};
		window.addEventListener(
			"agent-show-codex-thread-transcript",
			handleCodexThreadTranscriptEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-thread-transcript",
				handleCodexThreadTranscriptEvent,
			);
		};
	}, [showCodexThreadTranscript]);

	useEffect(() => {
		const handleCodexPermissionProfilesEvent = () => {
			void showCodexPermissionProfiles();
		};
		window.addEventListener(
			"agent-show-codex-permission-profiles",
			handleCodexPermissionProfilesEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-permission-profiles",
				handleCodexPermissionProfilesEvent,
			);
		};
	}, [showCodexPermissionProfiles]);

	useEffect(() => {
		const handleCodexHooksEvent = () => {
			void showCodexHooksReport();
		};
		window.addEventListener("agent-show-codex-hooks", handleCodexHooksEvent);
		return () => {
			window.removeEventListener(
				"agent-show-codex-hooks",
				handleCodexHooksEvent,
			);
		};
	}, [showCodexHooksReport]);

	useEffect(() => {
		const handleCodexRealtimeVoicesEvent = () => {
			void showCodexRealtimeVoicesReport();
		};
		window.addEventListener(
			"agent-show-codex-realtime-voices",
			handleCodexRealtimeVoicesEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-realtime-voices",
				handleCodexRealtimeVoicesEvent,
			);
		};
	}, [showCodexRealtimeVoicesReport]);

	useEffect(() => {
		const handleCodexMcpStatusEvent = () => {
			void showCodexMcpStatusReport();
		};
		window.addEventListener(
			"agent-show-codex-mcp-status",
			handleCodexMcpStatusEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-mcp-status",
				handleCodexMcpStatusEvent,
			);
		};
	}, [showCodexMcpStatusReport]);

	useEffect(() => {
		const handleCodexRuntimeConfigEvent = () => {
			void showCodexRuntimeConfigReport();
		};
		window.addEventListener(
			"agent-show-codex-runtime-config",
			handleCodexRuntimeConfigEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-runtime-config",
				handleCodexRuntimeConfigEvent,
			);
		};
	}, [showCodexRuntimeConfigReport]);

	useEffect(() => {
		const handleCodexRuntimeCapabilitiesEvent = () => {
			void showCodexRuntimeCapabilitiesReport();
		};
		window.addEventListener(
			"agent-show-codex-runtime-capabilities",
			handleCodexRuntimeCapabilitiesEvent,
		);
		return () => {
			window.removeEventListener(
				"agent-show-codex-runtime-capabilities",
				handleCodexRuntimeCapabilitiesEvent,
			);
		};
	}, [showCodexRuntimeCapabilitiesReport]);

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
		void invoke<ThreadSearchMatch[]>("search_agent_thread_messages", {
			request: {
				messages: session.messages,
				query,
			},
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
	}, [isSearchOpen, searchQuery, session.messages]);

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

	useEffect(() => {
		const handleFindEvent = () => {
			openThreadSearch();
		};
		window.addEventListener("agent-open-thread-find", handleFindEvent);
		return () =>
			window.removeEventListener("agent-open-thread-find", handleFindEvent);
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

	const selectedBackendLabel =
		backends.find((backend) => backend.id === selectedBackendId)?.name ??
		selectedBackendId ??
		"None";
	const latestTokenTotal =
		latestTokenUsage?.totalTokens ??
		(latestTokenUsage
			? latestTokenUsage.inputTokens + latestTokenUsage.outputTokens
			: null);
	const latestContextRemaining =
		latestTokenUsage?.contextWindowTokens != null && latestTokenTotal != null
			? Math.max(latestTokenUsage.contextWindowTokens - latestTokenTotal, 0)
			: null;
	const latestTokenUsageLabel = latestTokenUsage
		? [
				`in ${latestTokenUsage.inputTokens.toLocaleString()}`,
				`out ${latestTokenUsage.outputTokens.toLocaleString()}`,
				`used ${latestTokenTotal?.toLocaleString() ?? "unknown"}`,
				latestTokenUsage.contextWindowTokens != null
					? `context ${latestContextRemaining?.toLocaleString() ?? "unknown"} / ${latestTokenUsage.contextWindowTokens.toLocaleString()} left`
					: null,
			]
				.filter(Boolean)
				.join(" / ")
		: "unavailable";
	const codexAccountSummary =
		selectedBackendId === "codex"
			? (codexRuntimeStatus?.accountSummary ?? "unavailable")
			: null;
	const codexRateLimitSummary =
		selectedBackendId === "codex"
			? (codexRuntimeStatus?.rateLimitSummary ?? "unavailable")
			: null;

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
			sendMessageRef.current = (
				content: string,
				mentions?: MentionReference[],
			) => handleComposerSend(content, undefined, mentions);
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
								(p): p is ImagePart => p.type === "image",
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
									{onRewindToMessage && !isStreaming && (
										<RewindMessageButton
											onClick={() => void handleRewindToMessage(msg.id)}
										/>
									)}
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
								{onRewindToMessage && !isStreaming && (
									<RewindMessageButton
										onClick={() => void handleRewindToMessage(msg.id)}
									/>
								)}
								<AgentMessageParts
									msg={msg}
									isLastAgentStreaming={isLastAgentStreaming}
									worktreePath={worktreePath}
									showThinkingContent={showThinkingContent}
									rawScrollback={rawScrollback}
									respondPermission={onRespondPermission}
								/>
							</div>
						);
					})}
				</div>
			</div>
			<div className="shrink-0">
				{activeShellCommand && (
					<div className="px-3 pb-2">
						<div className="rounded border border-border bg-background px-3 py-2 text-xs">
							<div className="mb-2 flex items-center justify-between gap-2">
								<div className="min-w-0">
									<div className="font-medium">
										{activeShellCommand.background
											? "Shell background"
											: "Shell"}
										: {activeShellInfo?.status ?? "running"}
									</div>
									<div className="truncate font-mono text-muted-foreground">
										{activeShellCommand.command}
									</div>
								</div>
								{activeShellInfo?.status === "running" ||
								activeShellInfo?.status === "starting" ? (
									<button
										type="button"
										className="inline-flex h-6 shrink-0 items-center rounded px-2 text-muted-foreground hover:bg-muted hover:text-foreground"
										aria-label="Cancel shell command"
										onClick={() =>
											void cancelShellPty(activeShellCommand.ptyId)
										}
									>
										Cancel
									</button>
								) : null}
							</div>
							<pre className="max-h-36 overflow-y-auto whitespace-pre-wrap break-words rounded bg-muted/40 px-2 py-1.5 font-mono text-[11px] text-muted-foreground">
								{activeShellOutput || "No output yet"}
							</pre>
						</div>
					</div>
				)}
				{isStatusOpen && (
					<div className="px-3 pb-2">
						<div className="rounded border border-border bg-background px-3 py-2 text-xs">
							<div className="mb-2 flex items-center justify-between gap-2">
								<span className="font-medium">Session status</span>
								<button
									type="button"
									className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
									aria-label="Close session status"
									onClick={() => setIsStatusOpen(false)}
								>
									<X className="size-3.5" />
								</button>
							</div>
							<div className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 text-muted-foreground">
								<span>Session</span>
								<span className="min-w-0 truncate font-mono text-foreground">
									{session.id}
								</span>
								<span>Agent</span>
								<span className="text-foreground">
									{sessionStatus?.agent_state ?? "unavailable"}
								</span>
								<span>Turn</span>
								<span className="text-foreground">
									{sessionStatus?.turn_phase ?? "unavailable"}
								</span>
								<span>State</span>
								<span className="text-foreground">
									{sessionStatus?.session_state ?? session.state}
								</span>
								<span>Permission</span>
								<span className="text-foreground">
									{selectedPermissionProfileId
										? `Profile ${selectedPermissionProfileId}`
										: PERMISSION_MODE_LABELS[permissionMode]}
								</span>
								<span>Model</span>
								<span className="min-w-0 truncate text-foreground">
									{selectedModel || "None"}
								</span>
								<span>Backend</span>
								<span className="min-w-0 truncate text-foreground">
									{selectedBackendLabel}
								</span>
								<span>Queue</span>
								<span className="text-foreground">{pendingQueue.length}</span>
								<span>Latest tokens</span>
								<span className="text-foreground">{latestTokenUsageLabel}</span>
								{codexAccountSummary != null && (
									<>
										<span>Codex account</span>
										<span className="min-w-0 truncate text-foreground">
											{codexAccountSummary}
										</span>
									</>
								)}
								{codexRateLimitSummary != null && (
									<>
										<span>Codex limits</span>
										<span className="min-w-0 truncate text-foreground">
											{codexRateLimitSummary}
										</span>
									</>
								)}
							</div>
						</div>
					</div>
				)}
				{selectedBackendId === "codex" && codexGoal && (
					<div className="px-3 pb-2">
						<div className="flex min-w-0 items-center gap-2 rounded border border-border bg-background px-3 py-2 text-xs">
							<Target className="size-3.5 shrink-0 text-muted-foreground" />
							<div className="min-w-0 flex-1">
								<div className="flex min-w-0 items-center gap-2">
									<span className="shrink-0 font-medium">Goal</span>
									<span className="shrink-0 rounded border border-border/70 px-1.5 py-0.5 text-[10px] uppercase tracking-normal text-muted-foreground">
										{codexGoal.status}
									</span>
									<span className="min-w-0 truncate text-foreground">
										{codexGoal.objective}
									</span>
								</div>
								<div className="mt-1 truncate text-[11px] text-muted-foreground">
									Tokens {codexGoal.tokensUsed}
									{codexGoal.tokenBudget ? ` / ${codexGoal.tokenBudget}` : ""}
									{" · "}
									Elapsed {codexGoal.timeUsedSeconds}s
								</div>
							</div>
							<button
								type="button"
								className="inline-flex h-6 shrink-0 items-center rounded px-2 text-muted-foreground hover:bg-muted hover:text-foreground"
								onClick={() => void showCodexGoal()}
							>
								Edit
							</button>
							{codexGoal.status === "active" && (
								<button
									type="button"
									className="inline-flex h-6 shrink-0 items-center rounded px-2 text-muted-foreground hover:bg-muted hover:text-foreground"
									onClick={() =>
										void updateCodexGoalStatus(codexGoal, "paused")
									}
								>
									Pause
								</button>
							)}
							{codexGoal.status === "paused" && (
								<button
									type="button"
									className="inline-flex h-6 shrink-0 items-center rounded px-2 text-muted-foreground hover:bg-muted hover:text-foreground"
									onClick={() =>
										void updateCodexGoalStatus(codexGoal, "active")
									}
								>
									Resume
								</button>
							)}
							<button
								type="button"
								className="inline-flex h-6 shrink-0 items-center rounded px-2 text-muted-foreground hover:bg-muted hover:text-foreground"
								onClick={() => void clearCodexGoalFromRow()}
							>
								Clear
							</button>
						</div>
					</div>
				)}
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
									{nativeCommandNotice.showGoalEditor && (
										<div className="mt-2 space-y-2">
											<textarea
												value={goalDraft}
												onChange={(event) =>
													setGoalDraft(event.currentTarget.value)
												}
												className="min-h-20 w-full resize-y rounded border border-border bg-background px-2 py-1.5 text-xs text-foreground outline-none focus:border-primary"
												aria-label="Codex goal objective"
											/>
											<input
												type="number"
												min={1}
												value={goalTokenBudgetDraft}
												onChange={(event) =>
													setGoalTokenBudgetDraft(event.currentTarget.value)
												}
												className="h-7 w-full rounded border border-border bg-background px-2 font-mono text-[11px] text-foreground outline-none focus:border-primary"
												placeholder="Token budget"
												aria-label="Codex goal token budget"
											/>
											<div className="flex flex-wrap gap-1.5">
												<button
													type="button"
													className="inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
													onClick={() => void saveCodexGoal()}
												>
													Save goal
												</button>
												{nativeCommandNotice.codexGoal?.status === "active" && (
													<button
														type="button"
														className="inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
														onClick={() => void saveCodexGoal("paused")}
													>
														Pause
													</button>
												)}
												{nativeCommandNotice.codexGoal?.status === "paused" && (
													<button
														type="button"
														className="inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
														onClick={() => void saveCodexGoal("active")}
													>
														Resume
													</button>
												)}
												<button
													type="button"
													className="inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
													onClick={() => void clearCodexGoal()}
												>
													Clear
												</button>
											</div>
										</div>
									)}
									{nativeCommandNotice.permissionProfiles && (
										<div className="mt-2 flex flex-wrap gap-1.5">
											<button
												type="button"
												className={
													selectedPermissionProfileId
														? "inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
														: "inline-flex items-center gap-1 rounded border border-primary/50 bg-primary/10 px-2 py-1 text-[11px] text-foreground"
												}
												onClick={() => void applyCodexPermissionProfile(null)}
											>
												{!selectedPermissionProfileId && (
													<Check className="size-3 shrink-0" />
												)}
												<span>
													Use {PERMISSION_MODE_LABELS[permissionMode]}
												</span>
											</button>
											{nativeCommandNotice.permissionProfiles.map((profile) => {
												const selected =
													selectedPermissionProfileId === profile.id;
												return (
													<button
														type="button"
														key={profile.id}
														className={
															selected
																? "inline-flex max-w-full items-center gap-1 rounded border border-primary/50 bg-primary/10 px-2 py-1 text-[11px] text-foreground"
																: "inline-flex max-w-full items-center gap-1 rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
														}
														title={profile.description ?? profile.id}
														onClick={() =>
															void applyCodexPermissionProfile(profile.id)
														}
													>
														{selected && <Check className="size-3 shrink-0" />}
														<span className="truncate">{profile.id}</span>
													</button>
												);
											})}
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
									{nativeCommandNotice.exportTranscript && (
										<div className="mt-2 space-y-2">
											<div className="flex flex-wrap gap-1.5">
												<button
													type="button"
													className="inline-flex items-center gap-1 rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
													onClick={() => void copyExportTranscript()}
												>
													<Copy className="size-3 shrink-0" />
													<span>Copy transcript</span>
												</button>
												<button
													type="button"
													className="inline-flex items-center rounded border border-border/70 bg-background px-2 py-1 text-[11px] text-foreground hover:bg-muted"
													onClick={() => void writeExportTranscript()}
												>
													Write transcript
												</button>
											</div>
											<input
												ref={exportWritePathInputRef}
												type="text"
												value={exportWritePath}
												onChange={(event) =>
													setExportWritePath(event.currentTarget.value)
												}
												className="h-7 w-full rounded border border-border bg-background px-2 font-mono text-[11px] text-foreground outline-none focus:border-primary"
												placeholder="relative/transcript.md"
												aria-label="Transcript export write path"
											/>
										</div>
									)}
									{nativeCommandNotice.copyOptions &&
										nativeCommandNotice.copyOptions.length > 0 && (
											<div className="mt-2 space-y-2">
												<div className="flex flex-wrap gap-1.5">
													{nativeCommandNotice.copyOptions.map((option) => (
														<div
															key={option.id}
															className="inline-flex max-w-full items-center overflow-hidden rounded border border-border/70 bg-background"
														>
															<button
																type="button"
																className="inline-flex min-w-0 items-center gap-1 px-2 py-1 text-[11px] text-foreground hover:bg-muted"
																onClick={() =>
																	void copyNativeNoticeOption(option)
																}
															>
																<Copy className="size-3 shrink-0" />
																<span className="truncate">{option.label}</span>
															</button>
															<button
																type="button"
																className="border-l border-border/70 px-2 py-1 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground"
																onClick={() =>
																	void writeNativeNoticeOption(option)
																}
															>
																Write
															</button>
														</div>
													))}
												</div>
												<input
													ref={copyWritePathInputRef}
													type="text"
													value={copyWritePath}
													onChange={(event) => {
														setCopyWritePathEdited(true);
														setCopyWritePath(event.currentTarget.value);
													}}
													className="h-7 w-full rounded border border-border bg-background px-2 font-mono text-[11px] text-foreground outline-none focus:border-primary"
													placeholder="relative/path.ext"
													aria-label="Copy selection write path"
												/>
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
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<button
								type="button"
								className={`${activityStatus ? "" : "ml-auto"} inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground`}
								aria-label="Agent actions"
								title="Agent actions"
							>
								<MoreHorizontal className="size-3.5" />
							</button>
						</DropdownMenuTrigger>
						<DropdownMenuContent side="top" align="end">
							<DropdownMenuItem onSelect={() => void prepareCopyResponse()}>
								<Copy className="size-4" />
								<span>Copy response...</span>
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={() => void prepareTranscriptExport()}>
								<Download className="size-4" />
								<span>Export transcript</span>
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={() => void createAgentsGuidance()}>
								<FilePlus2 className="size-4" />
								<span>Create AGENTS.md</span>
							</DropdownMenuItem>
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void compactRuntimeContext()}>
									<Minimize2 className="size-4" />
									<span>Compact context</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void cleanCodexBackgroundTerminals()}
								>
									<X className="size-4" />
									<span>Clean Codex background terminals</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void runCodexShellCommand()}>
									<Terminal className="size-4" />
									<span>Run Codex shell command</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void startCodexReview()}>
									<Search className="size-4" />
									<span>Codex review uncommitted changes</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void startPromptedCodexReview("baseBranch")}
								>
									<Search className="size-4" />
									<span>Codex review base branch</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void startPromptedCodexReview("commit")}
								>
									<Search className="size-4" />
									<span>Codex review commit</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void startPromptedCodexReview("custom")}
								>
									<Search className="size-4" />
									<span>Codex review custom</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void showCodexGoal()}>
									<Target className="size-4" />
									<span>Codex goal</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexAccountStatus()}
								>
									<Gauge className="size-4" />
									<span>Codex account usage</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexThreadHistory()}
								>
									<History className="size-4" />
									<span>Codex thread history</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexThreadTranscript()}
								>
									<FileText className="size-4" />
									<span>Codex thread transcript</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void showCodexHooksReport()}>
									<Wrench className="size-4" />
									<span>Codex hooks</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexRealtimeVoicesReport()}
								>
									<Radio className="size-4" />
									<span>Codex realtime voices</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void startCodexRealtimeText()}
								>
									<Radio className="size-4" />
									<span>Start Codex realtime text</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void appendCodexRealtimeText()}
								>
									<Radio className="size-4" />
									<span>Append Codex realtime text</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem onSelect={() => void stopCodexRealtime()}>
									<X className="size-4" />
									<span>Stop Codex realtime</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexPermissionProfiles()}
								>
									<Check className="size-4" />
									<span>Codex permission profiles</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexMcpStatusReport()}
								>
									<Code2 className="size-4" />
									<span>Codex MCP status</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexRuntimeConfigReport()}
								>
									<Wrench className="size-4" />
									<span>Codex runtime config</span>
								</DropdownMenuItem>
							)}
							{selectedBackendId === "codex" && (
								<DropdownMenuItem
									onSelect={() => void showCodexRuntimeCapabilitiesReport()}
								>
									<Gauge className="size-4" />
									<span>Codex runtime capabilities</span>
								</DropdownMenuItem>
							)}
							<DropdownMenuItem onSelect={() => void showDebugConfigReport()}>
								<Wrench className="size-4" />
								<span>Debug config</span>
							</DropdownMenuItem>
							<DropdownMenuItem onSelect={() => void showDoctorReport()}>
								<Stethoscope className="size-4" />
								<span>Doctor</span>
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
					{session.messages.length > 0 && (
						<button
							type="button"
							className="inline-flex h-6 shrink-0 items-center gap-1 rounded px-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
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
						<div className="bg-destructive/10 text-destructive rounded-lg px-3 py-2 text-sm">
							{error}
						</div>
					</div>
				)}
				{pendingQueue.length > 0 && (
					<div className="px-3 pb-2 space-y-1">
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
				{queuedShellCommands.length > 0 && (
					<div className="px-3 pb-2 space-y-1">
						{queuedShellCommands.map((entry, index) => (
							<div
								key={entry.id}
								className="flex items-center gap-2 rounded border border-border bg-muted/40 px-2 py-1.5 text-xs"
							>
								<span className="shrink-0 text-muted-foreground">
									Queued shell {index + 1}
								</span>
								<span className="min-w-0 flex-1 truncate font-mono">
									{entry.prepared.displayCommand}
								</span>
								<button
									type="button"
									className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
									aria-label="Cancel queued shell command"
									onClick={() =>
										setQueuedShellCommands((commands) =>
											commands.filter((command) => command.id !== entry.id),
										)
									}
								>
									<X className="size-3.5" />
								</button>
							</div>
						))}
					</div>
				)}
				<MessageInput
					ref={messageInputRef}
					onSend={handleComposerSend}
					onInterrupt={onInterrupt}
					isStreaming={isStreaming}
					onCycleMode={cycleMode}
					mode={permissionMode}
					onModeChange={handlePermissionModeChange}
					models={availableModels}
					currentModelId={selectedModel}
					onModelChange={onModelChange}
					backends={backends}
					currentBackendId={selectedBackendId}
					onBackendChange={onBackendChange}
					backendDisabled={!canChangeBackend}
					worktreePath={worktreePath}
					chatSessionId={session.id}
					promptSuggestion={promptSuggestion}
					runtimeSlashCommands={runtimeSlashCommands}
				/>
			</div>
		</div>
	);
}
