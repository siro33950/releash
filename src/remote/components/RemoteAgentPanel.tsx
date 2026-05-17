import {
	Bot,
	CheckCircle2,
	Loader2,
	Play,
	RefreshCw,
	Send,
	Square,
	X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { BackendInfoMsg, WsMessage } from "@/types/protocol";
import type { MessagePart } from "@/types/session";
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
		case "system_notification":
			return part.detail ? `${part.label}: ${part.detail}` : part.label;
		case "image":
			return "[image]";
		default:
			return "";
	}
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
	} | null>(null);
	const [messages, setMessages] = useState<RemoteChatMessage[]>([]);
	const [draft, setDraft] = useState("");
	const [modelId, setModelId] = useState("");
	const [error, setError] = useState<string | null>(null);

	const availableBackends = useMemo(
		() => backends.filter((backend) => backend.available),
		[backends],
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
						});
						setMessages([]);
						setRunning(false);
						setError(null);
					} else {
						setError(
							msg.payload.error ?? "Agent session could not be started.",
						);
					}
					break;
				case "agent_message_response":
					setSending(false);
					if (msg.payload.success && msg.payload.session_id) {
						setStartedSession((current) => ({
							sessionId: msg.payload.session_id ?? current?.sessionId ?? "",
							backendId: msg.payload.backend_id ?? current?.backendId ?? null,
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
					if (!msg.payload.success) {
						setError(msg.payload.error ?? "Agent command failed.");
					} else if (msg.type === "agent_model_set_response") {
						setModelId(msg.payload.model_id ?? "");
						setError(null);
					}
					break;
			}
		});
	}, [subscribe, startedSession?.sessionId]);

	const lockedBackendId = startedSession?.backendId ?? selectedBackendId;
	const selectedBackend =
		availableBackends.find((backend) => backend.id === lockedBackendId) ??
		availableBackends[0] ??
		null;
	const selectedBackendModels = selectedBackend?.available_models ?? [];

	const startSession = () => {
		if (status !== "connected" || !selectedBackend) return;
		setStarting(true);
		setError(null);
		send({
			type: "agent_session_start_request",
			payload: {
				worktree_path: selectedWorktree,
				backend_id: selectedBackend.id,
			},
		});
	};

	const sendMessage = () => {
		const content = draft.trim();
		if (status !== "connected" || !content || sending) return;
		const localId = crypto.randomUUID();
		setMessages((current) => [
			...current,
			{ id: localId, role: "human", parts: [{ type: "text", content }] },
		]);
		setDraft("");
		setSending(true);
		setError(null);
		send({
			type: "agent_message_request",
			payload: {
				session_id: startedSession?.sessionId ?? null,
				worktree_path: selectedWorktree,
				content,
				permission_mode: "acceptEdits",
				backend_id: startedSession ? null : selectedBackend?.id,
			},
		});
	};

	const interrupt = () => {
		if (!startedSession) return;
		send({
			type: "agent_interrupt_request",
			payload: { session_id: startedSession.sessionId },
		});
	};

	const applyModel = () => {
		if (!startedSession) return;
		if (modelId.length === 0) return;
		send({
			type: "agent_model_set_request",
			payload: {
				session_id: startedSession.sessionId,
				model_id: modelId,
			},
		});
	};

	const clearModel = () => {
		if (!startedSession) return;
		send({
			type: "agent_model_set_request",
			payload: {
				session_id: startedSession.sessionId,
				model_id: null,
			},
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
					<span className="text-xs text-muted-foreground">Backend</span>
					<select
						className="w-full h-9 rounded border border-border bg-background px-2 text-sm"
						value={selectedBackend?.id ?? ""}
						onChange={(event) => onBackendChange(event.target.value || null)}
						disabled={
							availableBackends.length === 0 ||
							starting ||
							Boolean(startedSession)
						}
					>
						{availableBackends.map((backend) => (
							<option key={backend.id} value={backend.id}>
								{backend.name}
							</option>
						))}
					</select>
				</label>

				<button
					type="button"
					onClick={startSession}
					disabled={status !== "connected" || !selectedBackend || starting}
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

				{startedSession && (
					<div className="flex gap-2">
						<select
							value={modelId}
							onChange={(event) => setModelId(event.target.value)}
							className="min-w-0 flex-1 h-9 rounded border border-border bg-background px-2 text-sm"
							aria-label="Model"
						>
							{selectedBackendModels.map((model) => (
								<option key={model.value} value={model.value}>
									{model.value}
								</option>
							))}
						</select>
						<button
							type="button"
							onClick={applyModel}
							disabled={modelId.length === 0}
							className="px-3 h-9 rounded border border-border text-sm"
						>
							Set
						</button>
						<button
							type="button"
							onClick={clearModel}
							className="inline-flex items-center justify-center h-9 w-9 rounded border border-border"
							aria-label="Clear model"
						>
							<X className="size-4" />
						</button>
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
								? message.parts.map(renderPart).filter(Boolean).join("\n")
								: "Working..."}
						</div>
					))}
				</div>

				{error && (
					<div className="rounded border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
						{error}
					</div>
				)}
			</div>

			<div className="border-t border-border p-2">
				<div className="flex items-end gap-2">
					<textarea
						value={draft}
						onChange={(event) => setDraft(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && !event.shiftKey) {
								event.preventDefault();
								sendMessage();
							}
						}}
						className="min-h-10 max-h-28 flex-1 resize-none rounded border border-border bg-background px-2 py-2 text-sm"
						placeholder="Message"
						disabled={status !== "connected"}
					/>
					{sending || running ? (
						<button
							type="button"
							onClick={interrupt}
							className="inline-flex size-10 items-center justify-center rounded border border-border"
							aria-label="Interrupt agent"
						>
							<Square className="size-4" />
						</button>
					) : (
						<button
							type="button"
							onClick={sendMessage}
							disabled={status !== "connected" || !draft.trim()}
							className="inline-flex size-10 items-center justify-center rounded bg-primary text-primary-foreground disabled:opacity-50"
							aria-label="Send message"
						>
							<Send className="size-4" />
						</button>
					)}
				</div>
			</div>
		</div>
	);
}
