import {
	Bot,
	CheckCircle2,
	ImagePlus,
	Loader2,
	Play,
	RefreshCw,
	Send,
	Square,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
	AgentSessionSnapshot,
	BackendInfoMsg,
	WsMessage,
} from "@/types/protocol";
import {
	getModelInfoBackend,
	getModelInfoDisplayName,
	getModelInfoId,
	type ImageAttachment,
	type MentionReference,
	type MessagePart,
	normalizeModelSelectionId,
	normalizePermissionMode,
	PERMISSION_MODE_LABELS,
	PERMISSION_MODES,
	type PermissionMode,
	type QueuedAgentTurn,
	type SessionSummary,
	type SlashCommand,
} from "@/types/session";
import type { Subscribe } from "../hooks/useMessageBus";
import type { ConnectionStatus } from "../hooks/useWebSocket";

interface RemoteAgentPanelProps {
	selectedWorktree: string;
	backends: BackendInfoMsg[];
	selectedBackendId: string | null;
	backendLoading: boolean;
	status: ConnectionStatus;
	send: (msg: WsMessage) => void;
	subscribe: Subscribe;
	onBackendChange: (id: string | null) => void;
	onRefreshBackends: () => void;
}

interface RemoteAttachedImage {
	id: string;
	attachment: ImageAttachment;
	previewUrl: string;
}

interface RemoteChatMessage {
	id: string;
	role: "human" | "agent";
	parts: MessagePart[];
}

function renderPart(part: MessagePart): string {
	switch (part.type) {
		case "text":
		case "thinking":
		case "error":
			return part.content;
		case "tool_use":
			return `Using ${part.tool}`;
		case "tool_result":
			return part.content;
		case "permission":
			return `Permission ${part.status}: ${part.request.display_name ?? part.request.tool_name}`;
		case "task_status":
			return part.summary ?? part.description ?? `Task ${part.status}`;
		case "todo_list_snapshot": {
			const completed = part.items.filter((item) => item.completed).length;
			return `TODO ${completed}/${part.items.length}`;
		}
		case "system_notification":
			return part.detail ? `${part.label}: ${part.detail}` : part.label;
		case "image":
			return "[image]";
		default:
			return "";
	}
}

function remotePartKey(messageId: string, part: MessagePart): string {
	switch (part.type) {
		case "tool_use":
			return `${messageId}:tool_use:${part.id}`;
		case "tool_result":
			return `${messageId}:tool_result:${part.toolUseId ?? part.content}`;
		case "permission":
			return `${messageId}:permission:${part.request.request_id}`;
		case "task_status":
			return `${messageId}:task_status:${part.taskToolUseId}:${part.status}`;
		case "todo_list_snapshot":
			return `${messageId}:todo_list:${part.items.map((item) => `${item.completed}:${item.text}`).join("|")}`;
		case "system_notification":
			return `${messageId}:system:${part.notificationType}:${part.status}:${part.label}`;
		case "image":
			return `${messageId}:image:${part.mediaType}:${part.data.slice(0, 32)}`;
		default:
			return `${messageId}:${part.type}:${part.parentToolUseId ?? ""}:${part.content}`;
	}
}

const MENTION_SYNC_RE =
	/@([^ \t\r\n@:]+(?:\.[^ \t\r\n@:]+)*)(?::L(\d+)(?:-L(\d+))?)?/g;

function syncRemoteMentionsWithText(
	text: string,
	refs: MentionReference[],
): MentionReference[] | undefined {
	const available = new Map<string, number>();
	for (const ref of refs) {
		available.set(ref.filePath, (available.get(ref.filePath) ?? 0) + 1);
	}
	const synced: MentionReference[] = [];
	const re = new RegExp(MENTION_SYNC_RE.source, "g");
	for (;;) {
		const match = re.exec(text);
		if (match === null) break;
		const filePath = match[1];
		const remaining = available.get(filePath) ?? 0;
		if (remaining === 0) continue;
		const ref: MentionReference = { filePath };
		if (match[2]) ref.startLine = Number(match[2]);
		if (match[3]) ref.endLine = Number(match[3]);
		synced.push(ref);
		available.set(filePath, remaining - 1);
	}
	return synced.length > 0 ? synced : undefined;
}

function findRemoteMentionTrigger(
	text: string,
	cursorPos: number,
): { start: number; query: string } | null {
	for (let i = cursorPos - 1; i >= 0; i--) {
		const ch = text[i];
		if (ch === "@") {
			if (i === 0 || /\s/.test(text[i - 1])) {
				const query = text.slice(i + 1, cursorPos);
				if (!/\s/.test(query)) return { start: i, query };
			}
			return null;
		}
		if (/\s/.test(ch)) return null;
	}
	return null;
}

export function RemoteAgentPanel({
	selectedWorktree,
	backends,
	selectedBackendId,
	backendLoading,
	status,
	send,
	subscribe,
	onBackendChange,
	onRefreshBackends,
}: RemoteAgentPanelProps) {
	const [starting, setStarting] = useState(false);
	const [sending, setSending] = useState(false);
	const [running, setRunning] = useState(false);
	const [startedSession, setStartedSession] = useState<{
		sessionId: string;
		backendId: string | null;
		agentSessionId: string | null;
	} | null>(null);
	const [sessionSummaries, setSessionSummaries] = useState<SessionSummary[]>(
		[],
	);
	const [messages, setMessages] = useState<RemoteChatMessage[]>([]);
	const [pendingQueue, setPendingQueue] = useState<QueuedAgentTurn[]>([]);
	const [slashCommands, setSlashCommands] = useState<SlashCommand[]>([]);
	const [draft, setDraft] = useState("");
	const [attachedImages, setAttachedImages] = useState<RemoteAttachedImage[]>(
		[],
	);
	const [pendingImageCount, setPendingImageCount] = useState(0);
	const [slashDismissed, setSlashDismissed] = useState(false);
	const [slashSelectedIndex, setSlashSelectedIndex] = useState(0);
	const [mentionDismissed, setMentionDismissed] = useState(false);
	const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);
	const [mentionFiles, setMentionFiles] = useState<string[]>([]);
	const [mentionTrigger, setMentionTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);
	const [mentionRefs, setMentionRefs] = useState<MentionReference[]>([]);
	const [modelId, setModelId] = useState("");
	const [permissionMode, setPermissionMode] = useState<PermissionMode>("edit");
	const [error, setError] = useState<string | null>(null);
	const imageInputRef = useRef<HTMLInputElement>(null);
	const latestMentionRequestIdRef = useRef<string | null>(null);

	const availableBackends = useMemo(
		() => backends.filter((backend) => backend.available),
		[backends],
	);
	const modelEntries = useMemo(
		() =>
			availableBackends.flatMap((backend) =>
				backend.available_models.map((model) => ({
					...model,
					backend: getModelInfoBackend(model) || backend.id,
				})),
			),
		[availableBackends],
	);

	const applySessionSnapshot = useCallback(
		(snapshot: AgentSessionSnapshot | null) => {
			if (!snapshot) {
				setStartedSession(null);
				setMessages([]);
				setPendingQueue([]);
				setRunning(false);
				setModelId("");
				return;
			}
			setStartedSession({
				sessionId: snapshot.id,
				backendId: snapshot.backendId ?? null,
				agentSessionId: snapshot.agentSessionId ?? null,
			});
			setMessages(
				snapshot.messages.map((message) => ({
					id: message.id,
					role: message.role === "human" ? "human" : "agent",
					parts:
						message.parts ??
						(message.content
							? [{ type: "text", content: message.content }]
							: []),
				})),
			);
			setPendingQueue(snapshot.pendingQueue ?? []);
			setRunning(
				snapshot.turnPhase === "streaming" ||
					snapshot.turnPhase === "waiting_permission",
			);
			setPermissionMode(normalizePermissionMode(snapshot.permissionMode));
			setModelId(
				normalizeModelSelectionId(
					snapshot.availableModels ?? [],
					snapshot.selectedModel,
				),
			);
		},
		[],
	);

	useEffect(() => {
		return subscribe((msg) => {
			switch (msg.type) {
				case "agent_session_start_response":
					setStarting(false);
					if (msg.payload.success && msg.payload.session_id) {
						setStartedSession({
							sessionId: msg.payload.session_id,
							backendId: msg.payload.backend_id ?? null,
							agentSessionId: null,
						});
						setMessages([]);
						setPendingQueue([]);
						setAttachedImages([]);
						setPendingImageCount(0);
						setMentionRefs([]);
						setRunning(false);
						setError(null);
					} else {
						setError(
							msg.payload.error ?? "Agent session could not be started.",
						);
					}
					break;
				case "agent_sessions_response":
					if (msg.payload.worktree_path !== selectedWorktree) break;
					if (msg.payload.success) {
						setSessionSummaries(msg.payload.sessions);
						applySessionSnapshot(msg.payload.active_session ?? null);
						setError(null);
					} else {
						setSessionSummaries([]);
						applySessionSnapshot(null);
						setError(
							msg.payload.error ?? "Agent sessions could not be loaded.",
						);
					}
					break;
				case "agent_session_get_response":
					if (!msg.payload.success) {
						setError(msg.payload.error ?? "Agent session could not be loaded.");
					} else {
						const snapshot = msg.payload.session ?? null;
						if (snapshot && snapshot.worktreePath !== selectedWorktree) break;
						applySessionSnapshot(snapshot);
						setError(null);
					}
					break;
				case "agent_message_response":
					setSending(false);
					if (msg.payload.success && msg.payload.session_id) {
						setStartedSession((current) => ({
							sessionId: msg.payload.session_id ?? current?.sessionId ?? "",
							backendId: msg.payload.backend_id ?? current?.backendId ?? null,
							agentSessionId: current?.agentSessionId ?? null,
						}));
						if (msg.payload.agent_message_id) {
							setRunning(true);
							setMessages((current) => [
								...current,
								{
									id: msg.payload.agent_message_id ?? crypto.randomUUID(),
									role: "agent",
									parts: [],
								},
							]);
						}
						setPendingQueue(msg.payload.pending_queue ?? []);
						if (msg.payload.sessions) {
							setSessionSummaries(msg.payload.sessions);
						}
						setError(null);
					} else {
						setError(msg.payload.error ?? "Message could not be sent.");
					}
					break;
				case "agent_stream_sync":
					if (msg.payload.session_id !== startedSession?.sessionId) {
						break;
					}
					setMessages((current) => {
						// Rust sends the cumulative `streaming_parts` on every emit, so the
						// receiver replaces the message state wholesale. Replays / partial
						// failures collapse to the same final state without double-merging.
						let found = false;
						const next = current.map((message) => {
							if (message.id !== msg.payload.message_id) return message;
							found = true;
							return {
								...message,
								parts: msg.payload.parts,
							};
						});
						if (found) return next;
						return [
							...next,
							{
								id: msg.payload.message_id,
								role: "agent",
								parts: msg.payload.parts,
							},
						];
					});
					break;
				case "agent_state_sync":
					if (
						msg.payload.session_id &&
						msg.payload.session_id === startedSession?.sessionId
					) {
						setRunning(
							msg.payload.state === "running" ||
								msg.payload.state === "waiting",
						);
					}
					break;
				case "agent_interrupt_response":
				case "agent_model_set_response":
				case "agent_queue_cancel_response":
				case "agent_permission_response_response":
					if (!msg.payload.success) {
						setError(msg.payload.error ?? "Agent command failed.");
					} else if (msg.type === "agent_queue_cancel_response") {
						setPendingQueue(msg.payload.pending_queue ?? []);
						setError(null);
					} else if (msg.type === "agent_model_set_response") {
						const nextModelId = normalizeModelSelectionId(
							modelEntries,
							msg.payload.model_id,
						);
						setModelId(nextModelId);
						const nextModel = modelEntries.find(
							(model) => getModelInfoId(model) === nextModelId,
						);
						const nextBackendId = nextModel
							? getModelInfoBackend(nextModel)
							: null;
						if (nextBackendId) {
							setStartedSession((current) =>
								current ? { ...current, backendId: nextBackendId } : current,
							);
						}
						setError(null);
					} else if (msg.type === "agent_permission_response_response") {
						setError(null);
					}
					break;
				case "agent_slash_commands_response":
					if (msg.payload.worktree_path === selectedWorktree) {
						setSlashCommands(msg.payload.success ? msg.payload.commands : []);
					}
					break;
				case "agent_mention_files_response":
					if (
						msg.payload.request_id === latestMentionRequestIdRef.current &&
						msg.payload.worktree_path === selectedWorktree
					) {
						setMentionFiles(msg.payload.success ? msg.payload.files : []);
					}
					break;
				case "agent_image_prepare_response":
					setPendingImageCount((count) => Math.max(0, count - 1));
					if (msg.payload.success && msg.payload.attachment) {
						const attachment = msg.payload.attachment;
						setAttachedImages((current) => [
							...current,
							{
								id: msg.payload.request_id,
								attachment,
								previewUrl: `data:${attachment.mediaType};base64,${attachment.data}`,
							},
						]);
						setError(null);
					} else {
						setError(msg.payload.error ?? "Image could not be attached.");
					}
					break;
				case "agent_permission_mode_set_response":
					if (!msg.payload.success) {
						setError(
							msg.payload.error ?? "Permission mode could not be changed.",
						);
					} else {
						setError(null);
					}
					break;
			}
		});
	}, [
		subscribe,
		startedSession?.sessionId,
		selectedWorktree,
		applySessionSnapshot,
		modelEntries,
	]);

	useEffect(() => {
		setStarting(false);
		setSending(false);
		setSessionSummaries([]);
		applySessionSnapshot(null);
		setDraft("");
		setAttachedImages([]);
		setPendingImageCount(0);
		setMentionRefs([]);
		setMentionTrigger(null);
		setMentionFiles([]);
		setError(null);
		if (status !== "connected") return;
		send({
			type: "agent_sessions_request",
			payload: { worktree_path: selectedWorktree },
		});
	}, [status, selectedWorktree, send, applySessionSnapshot]);

	useEffect(() => {
		setSlashCommands([]);
		if (status !== "connected") return;
		send({
			type: "agent_slash_commands_request",
			payload: { worktree_path: selectedWorktree },
		});
	}, [status, selectedWorktree, send]);

	const mentionQuery = mentionTrigger?.query ?? null;

	useEffect(() => {
		if (mentionQuery === null || mentionDismissed || status !== "connected") {
			setMentionFiles([]);
			return;
		}
		setMentionFiles([]);
		setMentionSelectedIndex(0);
		const requestId = crypto.randomUUID();
		latestMentionRequestIdRef.current = requestId;
		const timer = setTimeout(() => {
			send({
				type: "agent_mention_files_request",
				payload: {
					request_id: requestId,
					worktree_path: selectedWorktree,
					query: mentionQuery,
				},
			});
		}, 150);
		return () => clearTimeout(timer);
	}, [mentionQuery, mentionDismissed, status, selectedWorktree, send]);

	const selectedModelId = normalizeModelSelectionId(modelEntries, modelId);
	const selectedModel =
		modelEntries.find((model) => getModelInfoId(model) === selectedModelId) ??
		null;
	const selectedModelBackendId = selectedModel
		? getModelInfoBackend(selectedModel)
		: null;
	const canChangeBackend =
		!startedSession ||
		(!running && messages.length === 0 && !startedSession.agentSessionId);
	const lockedBackendId =
		(canChangeBackend ? selectedModelBackendId : null) ??
		startedSession?.backendId ??
		selectedBackendId;
	const selectedBackend =
		availableBackends.find((backend) => backend.id === lockedBackendId) ??
		availableBackends[0] ??
		null;
	const showSlashPopup =
		draft.startsWith("/") && !draft.includes(" ") && !slashDismissed;
	const slashQuery = draft.slice(1).toLowerCase();
	const filteredSlashCommands = useMemo(() => {
		if (!showSlashPopup) return [];
		if (slashQuery.length === 0) return slashCommands;
		return slashCommands.filter((command) =>
			command.name.toLowerCase().startsWith(slashQuery),
		);
	}, [showSlashPopup, slashQuery, slashCommands]);
	const slashPopupOpen = filteredSlashCommands.length > 0;
	const mentionPopupOpen =
		!mentionDismissed && mentionTrigger !== null && mentionFiles.length > 0;

	useEffect(() => {
		if (slashSelectedIndex < filteredSlashCommands.length) return;
		setSlashSelectedIndex(Math.max(0, filteredSlashCommands.length - 1));
	}, [slashSelectedIndex, filteredSlashCommands.length]);

	// モデル未選択(null/Unset)状態は廃止。session 起動後にモデル候補があれば
	// デフォルト = 先頭モデルを選択状態にしておき、null を送る経路を残さない。
	useEffect(() => {
		if (modelId.length > 0) return;
		const first = modelEntries[0];
		if (!first) return;
		const firstModelId = getModelInfoId(first);
		setModelId(firstModelId);
		if (!startedSession) {
			onBackendChange(getModelInfoBackend(first) || null);
		}
	}, [startedSession, modelId, modelEntries, onBackendChange]);

	const selectModel = (nextModelId: string) => {
		setModelId(nextModelId);
		if (startedSession) return;
		const nextModel = modelEntries.find(
			(model) => getModelInfoId(model) === nextModelId,
		);
		onBackendChange(nextModel ? getModelInfoBackend(nextModel) || null : null);
	};

	const startSession = () => {
		if (status !== "connected" || !selectedBackend) return;
		setStarting(true);
		setError(null);
		const modelBackendId = selectedModelBackendId ?? selectedBackend.id;
		const payload: Extract<
			WsMessage,
			{ type: "agent_session_start_request" }
		>["payload"] = {
			worktree_path: selectedWorktree,
			backend_id: modelBackendId,
			permission_mode: permissionMode,
		};
		if (selectedModelId) {
			payload.model_id = selectedModelId;
		}
		send({
			type: "agent_session_start_request",
			payload,
		});
	};

	const sendMessage = () => {
		const content = draft.trim();
		const images = attachedImages.map((image) => image.attachment);
		const mentions = syncRemoteMentionsWithText(content, mentionRefs);
		if (
			status !== "connected" ||
			(!content && images.length === 0) ||
			sending
		) {
			return;
		}
		const localId = crypto.randomUUID();
		const localParts: MessagePart[] = [];
		if (content) localParts.push({ type: "text", content });
		for (const image of images) {
			localParts.push({
				type: "image",
				data: image.data,
				mediaType: image.mediaType,
			});
		}
		setMessages((current) => [
			...current,
			{ id: localId, role: "human", parts: localParts },
		]);
		setDraft("");
		setAttachedImages([]);
		setMentionRefs([]);
		setMentionTrigger(null);
		setMentionDismissed(false);
		setSending(true);
		setError(null);
		const payload: Extract<
			WsMessage,
			{ type: "agent_message_request" }
		>["payload"] = {
			session_id: startedSession?.sessionId ?? null,
			worktree_path: selectedWorktree,
			content,
			permission_mode: permissionMode,
			backend_id: startedSession ? null : selectedBackend?.id,
		};
		if (!startedSession && selectedModelId) {
			payload.model_id = selectedModelId;
		}
		if (images.length > 0) {
			payload.images = images;
		}
		if (mentions && mentions.length > 0) {
			payload.mentions = mentions;
		}
		send({
			type: "agent_message_request",
			payload,
		});
	};

	const selectSlashCommand = (command: SlashCommand) => {
		setDraft(`/${command.name} `);
		setSlashDismissed(true);
		setSlashSelectedIndex(0);
	};

	const selectMention = (filePath: string) => {
		if (!mentionTrigger) return;
		const before = draft.slice(0, mentionTrigger.start);
		const after = draft
			.slice(mentionTrigger.start + 1 + mentionTrigger.query.length)
			.replace(/^\s/, "");
		setDraft(`${before}@${filePath} ${after}`);
		setMentionRefs((current) => [
			...current,
			{ filePath, startLine: undefined, endLine: undefined },
		]);
		setMentionTrigger(null);
		setMentionDismissed(true);
		setMentionSelectedIndex(0);
	};

	const requestImagePreparation = async (files: File[]) => {
		for (const file of files) {
			if (!file.type.startsWith("image/")) continue;
			const requestId = crypto.randomUUID();
			const bytes = new Uint8Array(await file.arrayBuffer());
			setPendingImageCount((count) => count + 1);
			send({
				type: "agent_image_prepare_request",
				payload: {
					request_id: requestId,
					data: Array.from(bytes),
				},
			});
		}
	};

	const removeImage = (id: string) => {
		setAttachedImages((current) => current.filter((image) => image.id !== id));
	};

	const interrupt = () => {
		if (!startedSession) return;
		send({
			type: "agent_interrupt_request",
			payload: { session_id: startedSession.sessionId },
		});
	};

	const respondPermission = (requestId: string, allow: boolean) => {
		if (!startedSession) return;
		send({
			type: "agent_permission_response_request",
			payload: {
				session_id: startedSession.sessionId,
				request_id: requestId,
				behavior: allow ? "allow" : "deny",
				message: allow ? null : "User denied",
				updated_input: null,
			},
		});
	};

	const cancelQueuedTurn = (queuedTurnId: string) => {
		if (!startedSession) return;
		send({
			type: "agent_queue_cancel_request",
			payload: {
				session_id: startedSession.sessionId,
				queued_turn_id: queuedTurnId,
			},
		});
	};

	const applyModel = () => {
		if (!startedSession) return;
		if (selectedModelId.length === 0) return;
		send({
			type: "agent_model_set_request",
			payload: {
				session_id: startedSession.sessionId,
				model_id: selectedModelId,
			},
		});
	};

	const changePermissionMode = (mode: PermissionMode) => {
		setPermissionMode(mode);
		if (!startedSession) return;
		setError(null);
		send({
			type: "agent_permission_mode_set_request",
			payload: {
				session_id: startedSession.sessionId,
				permission_mode: mode,
			},
		});
	};

	const selectSession = (sessionId: string) => {
		if (!sessionId) {
			applySessionSnapshot(null);
			return;
		}
		send({
			type: "agent_session_get_request",
			payload: { session_id: sessionId },
		});
	};

	return (
		<div className="flex flex-col h-full bg-background">
			<div className="flex items-center justify-between px-3 py-2 border-b border-border">
				<div className="flex items-center gap-2 min-w-0">
					<Bot className="size-4 text-muted-foreground shrink-0" />
					<span className="text-xs font-semibold text-muted-foreground uppercase tracking-wider">
						Agent
					</span>
				</div>
				<button
					type="button"
					onClick={onRefreshBackends}
					className="p-1 hover:bg-muted rounded transition-colors"
					aria-label="Refresh backends"
					disabled={backendLoading}
				>
					<RefreshCw
						className={`size-3.5 text-muted-foreground ${backendLoading ? "animate-spin" : ""}`}
					/>
				</button>
			</div>

			<div className="flex-1 overflow-y-auto p-3 space-y-3">
				<label className="block space-y-1">
					<span className="text-xs text-muted-foreground">Model</span>
					<select
						className="w-full h-9 rounded border border-border bg-background px-2 text-sm"
						value={selectedModelId}
						onChange={(event) => selectModel(event.target.value)}
						disabled={modelEntries.length === 0 || starting || backendLoading}
					>
						{modelEntries.map((model) => {
							const modelEntryId = getModelInfoId(model);
							const backendId = getModelInfoBackend(model);
							return (
								<option
									key={modelEntryId}
									value={modelEntryId}
									disabled={
										!canChangeBackend &&
										backendId !== "" &&
										backendId !== startedSession?.backendId
									}
								>
									{getModelInfoDisplayName(model)}
								</option>
							);
						})}
					</select>
				</label>

				{selectedBackend && (
					<div className="text-xs text-muted-foreground">
						Provider: {selectedBackend.name}
					</div>
				)}

				{startedSession && (
					<div className="flex gap-2">
						<button
							type="button"
							onClick={applyModel}
							disabled={modelEntries.length > 0 && selectedModelId.length === 0}
							className="w-full h-9 rounded border border-border text-sm disabled:opacity-50"
						>
							Set
						</button>
					</div>
				)}

				<label className="block space-y-1">
					<span className="text-xs text-muted-foreground">Session</span>
					<select
						className="w-full h-9 rounded border border-border bg-background px-2 text-sm"
						value={startedSession?.sessionId ?? ""}
						onChange={(event) => selectSession(event.target.value)}
						aria-label="Agent session"
					>
						<option value="">New Session</option>
						{sessionSummaries.map((session) => (
							<option key={session.id} value={session.id}>
								{session.firstMessage || "Untitled"} ({session.messageCount})
							</option>
						))}
					</select>
				</label>

				<label className="block space-y-1">
					<span className="text-xs text-muted-foreground">Permission</span>
					<select
						className="w-full h-9 rounded border border-border bg-background px-2 text-sm"
						value={permissionMode}
						onChange={(event) =>
							changePermissionMode(event.target.value as PermissionMode)
						}
						data-testid="remote-permission-mode-select"
					>
						{PERMISSION_MODES.map((mode) => (
							<option key={mode} value={mode}>
								{PERMISSION_MODE_LABELS[mode]}
							</option>
						))}
					</select>
				</label>

				<button
					type="button"
					onClick={startSession}
					disabled={
						status !== "connected" ||
						!selectedBackend ||
						(modelEntries.length > 0 && selectedModelId.length === 0) ||
						starting
					}
					className="inline-flex items-center justify-center gap-2 w-full h-9 rounded bg-primary text-primary-foreground text-sm font-medium disabled:opacity-50"
				>
					{starting ? (
						<Loader2 className="size-4 animate-spin" />
					) : (
						<Play className="size-4" />
					)}
					Start Session
				</button>

				{startedSession && (
					<div className="flex items-start gap-2 rounded border border-success/30 bg-success/10 p-3 text-sm text-success">
						<CheckCircle2 className="size-4 mt-0.5 shrink-0" />
						<div className="min-w-0">
							<div className="font-medium">Session ready</div>
							<div className="text-xs truncate">
								{startedSession.backendId ?? "default"} /{" "}
								{startedSession.sessionId}
							</div>
						</div>
					</div>
				)}

				<div className="space-y-2">
					{messages.map((message) => (
						<div
							key={message.id}
							className={`rounded border border-border p-2 text-sm whitespace-pre-wrap ${
								message.role === "human" ? "bg-muted/50" : "bg-background"
							}`}
						>
							<div className="mb-1 text-[10px] uppercase text-muted-foreground">
								{message.role === "human" ? "You" : "Agent"}
							</div>
							{message.parts.length > 0
								? message.parts.map((part) => {
										const partKey = remotePartKey(message.id, part);
										if (part.type === "image") {
											return (
												<img
													key={partKey}
													src={`data:${part.mediaType};base64,${part.data}`}
													alt="Attached"
													className="mt-1 h-24 max-w-full rounded border object-contain"
												/>
											);
										}
										if (part.type !== "permission") {
											return <div key={partKey}>{renderPart(part)}</div>;
										}
										const label =
											part.request.display_name ??
											part.request.tool_name ??
											"Permission";
										return (
											<div
												key={partKey}
												className="space-y-2 rounded border border-warning/30 bg-warning/10 p-2"
											>
												<div>
													<div className="font-medium">{label}</div>
													{part.request.description ? (
														<div className="text-xs text-muted-foreground">
															{part.request.description}
														</div>
													) : null}
												</div>
												{part.status === "pending" ? (
													<div className="flex gap-2">
														<button
															type="button"
															onClick={() =>
																respondPermission(part.request.request_id, true)
															}
															className="h-8 rounded bg-primary px-3 text-xs font-medium text-primary-foreground"
														>
															Allow
														</button>
														<button
															type="button"
															onClick={() =>
																respondPermission(
																	part.request.request_id,
																	false,
																)
															}
															className="h-8 rounded border border-border px-3 text-xs font-medium"
														>
															Deny
														</button>
													</div>
												) : (
													<div className="text-xs text-muted-foreground">
														{part.status === "allowed" ? "Allowed" : "Denied"}
													</div>
												)}
											</div>
										);
									})
								: "Working..."}
						</div>
					))}
				</div>

				{error && (
					<div className="rounded border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
						{error}
					</div>
				)}

				{pendingQueue.length > 0 && (
					<div className="space-y-1">
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
									onClick={() => cancelQueuedTurn(turn.id)}
									className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
									aria-label="Cancel queued message"
								>
									<X className="size-3.5" />
								</button>
							</div>
						))}
					</div>
				)}
			</div>

			<div className="relative border-t border-border p-2">
				{mentionPopupOpen && (
					<div
						className="absolute bottom-full left-2 right-2 z-30 mb-2 max-h-48 overflow-y-auto rounded border border-border bg-popover p-1 shadow-lg"
						data-testid="remote-mention-file-list"
					>
						{mentionFiles.map((file, index) => (
							<button
								type="button"
								key={file}
								onClick={() => selectMention(file)}
								className={`flex w-full items-center rounded px-2 py-1.5 text-left font-mono text-xs ${
									index === mentionSelectedIndex
										? "bg-accent text-accent-foreground"
										: "hover:bg-accent/70"
								}`}
							>
								<span className="truncate">{file}</span>
							</button>
						))}
					</div>
				)}
				{slashPopupOpen && !mentionPopupOpen && (
					<div
						className="absolute bottom-full left-2 right-2 z-20 mb-2 max-h-48 overflow-y-auto rounded border border-border bg-popover p-1 shadow-lg"
						data-testid="remote-slash-command-list"
					>
						{filteredSlashCommands.map((command, index) => (
							<button
								type="button"
								key={command.name}
								onClick={() => selectSlashCommand(command)}
								className={`flex w-full flex-col items-start rounded px-2 py-1.5 text-left text-sm ${
									index === slashSelectedIndex
										? "bg-accent text-accent-foreground"
										: "hover:bg-accent/70"
								}`}
							>
								<span className="font-medium">
									/{command.name}
									{command.argumentHint ? (
										<span className="ml-1 font-normal text-muted-foreground">
											{command.argumentHint}
										</span>
									) : null}
								</span>
								<span className="text-xs text-muted-foreground">
									{command.description}
								</span>
							</button>
						))}
					</div>
				)}
				{(attachedImages.length > 0 || pendingImageCount > 0) && (
					<div
						className="mb-2 flex flex-wrap gap-2"
						data-testid="remote-image-preview-list"
					>
						{attachedImages.map((image) => (
							<div
								key={image.id}
								className="group relative"
								data-testid="remote-image-preview-item"
							>
								<img
									src={image.previewUrl}
									alt="Attached"
									className="h-14 w-14 rounded border object-cover"
								/>
								<button
									type="button"
									onClick={() => removeImage(image.id)}
									className="absolute -right-1.5 -top-1.5 inline-flex size-5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 focus:opacity-100"
									aria-label="Remove image"
								>
									<X className="size-3" />
								</button>
							</div>
						))}
						{pendingImageCount > 0 && (
							<div className="inline-flex h-14 w-14 items-center justify-center rounded border border-border bg-muted text-muted-foreground">
								<Loader2 className="size-4 animate-spin" />
							</div>
						)}
					</div>
				)}
				<div className="flex items-end gap-2">
					<input
						ref={imageInputRef}
						type="file"
						accept="image/*"
						multiple
						className="hidden"
						onChange={(event) => {
							const files = Array.from(event.target.files ?? []);
							event.target.value = "";
							void requestImagePreparation(files);
						}}
					/>
					<textarea
						value={draft}
						onChange={(event) => {
							const value = event.target.value;
							setDraft(value);
							setSlashDismissed(false);
							setSlashSelectedIndex(0);
							const cursorPos = event.target.selectionStart ?? value.length;
							const trigger = findRemoteMentionTrigger(value, cursorPos);
							if (trigger) {
								setMentionTrigger(trigger);
								setMentionDismissed(false);
							} else {
								setMentionTrigger(null);
							}
						}}
						onKeyDown={(event) => {
							if (mentionPopupOpen) {
								if (event.key === "ArrowDown") {
									event.preventDefault();
									setMentionSelectedIndex(
										(current) => (current + 1) % mentionFiles.length,
									);
									return;
								}
								if (event.key === "ArrowUp") {
									event.preventDefault();
									setMentionSelectedIndex(
										(current) =>
											(current - 1 + mentionFiles.length) % mentionFiles.length,
									);
									return;
								}
								if (event.key === "Enter" || event.key === "Tab") {
									event.preventDefault();
									selectMention(mentionFiles[mentionSelectedIndex]);
									return;
								}
								if (event.key === "Escape") {
									event.preventDefault();
									setMentionDismissed(true);
									return;
								}
							}
							if (slashPopupOpen) {
								if (event.key === "ArrowDown") {
									event.preventDefault();
									setSlashSelectedIndex(
										(current) => (current + 1) % filteredSlashCommands.length,
									);
									return;
								}
								if (event.key === "ArrowUp") {
									event.preventDefault();
									setSlashSelectedIndex(
										(current) =>
											(current - 1 + filteredSlashCommands.length) %
											filteredSlashCommands.length,
									);
									return;
								}
								if (event.key === "Enter") {
									event.preventDefault();
									selectSlashCommand(filteredSlashCommands[slashSelectedIndex]);
									return;
								}
								if (event.key === "Escape") {
									event.preventDefault();
									setSlashDismissed(true);
									return;
								}
							}
							if (event.key === "Enter" && !event.shiftKey) {
								event.preventDefault();
								sendMessage();
							}
						}}
						onPaste={(event) => {
							const files = Array.from(event.clipboardData.files).filter(
								(file) => file.type.startsWith("image/"),
							);
							if (files.length > 0) {
								event.preventDefault();
								void requestImagePreparation(files);
							}
						}}
						className="min-h-10 max-h-28 flex-1 resize-none rounded border border-border bg-background px-2 py-2 text-sm"
						placeholder="Message"
						disabled={status !== "connected"}
					/>
					<button
						type="button"
						onClick={() => imageInputRef.current?.click()}
						className="inline-flex size-10 items-center justify-center rounded border border-border"
						aria-label="Attach image"
						disabled={status !== "connected"}
					>
						<ImagePlus className="size-4" />
					</button>
					{running && (
						<button
							type="button"
							onClick={interrupt}
							className="inline-flex size-10 items-center justify-center rounded border border-border"
							aria-label="Interrupt agent"
						>
							<Square className="size-4" />
						</button>
					)}
					<button
						type="button"
						onClick={sendMessage}
						disabled={
							status !== "connected" ||
							(!draft.trim() && attachedImages.length === 0) ||
							sending ||
							pendingImageCount > 0
						}
						className="inline-flex size-10 items-center justify-center rounded bg-primary text-primary-foreground disabled:opacity-50"
						aria-label="Send message"
					>
						{sending ? (
							<Loader2 className="size-4 animate-spin" />
						) : (
							<Send className="size-4" />
						)}
					</button>
				</div>
			</div>
		</div>
	);
}
