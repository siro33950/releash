import { Loader2 } from "lucide-react";
import { useEffect, useRef } from "react";
import { useAgentChat } from "@/hooks/useAgentChat";
import { ActivityLog } from "./ActivityLog";
import { MessageInput } from "./MessageInput";
import { ModeSelector } from "./ModeSelector";
import { PermissionDialog } from "./PermissionDialog";
import { SessionList } from "./SessionList";
import { StreamMessage } from "./StreamMessage";
import { ThinkingIndicator } from "./ThinkingIndicator";

interface AgentChatPanelProps {
	worktreePath: string;
}

export function AgentChatPanel({ worktreePath }: AgentChatPanelProps) {
	const {
		sessions,
		activeSession,
		isStreaming,
		error,
		permissionMode,
		pendingPermission,
		sendMessage,
		interrupt,
		selectSession,
		refreshSessions,
		clearActiveSession,
		setPermissionMode,
		respondPermission,
	} = useAgentChat(worktreePath);

	const isWaiting = isStreaming && pendingPermission !== null;

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
	const lastAgentContent =
		agentMessages?.[agentMessages.length - 1]?.content?.length ?? 0;

	// biome-ignore lint/correctness/useExhaustiveDependencies: lastAgentContent triggers scroll on content growth
	useEffect(() => {
		if (!isStreaming) return;
		const el = scrollRef.current;
		if (!el) return;
		const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 100;
		if (isNearBottom) {
			el.scrollTop = el.scrollHeight;
		}
	}, [isStreaming, lastAgentContent]);

	const handleNewSession = () => {
		clearActiveSession();
		refreshSessions();
	};

	const msgs = activeSession?.messages;
	const lastMsg = msgs?.[msgs.length - 1];
	const showWaitingIndicator =
		isStreaming &&
		lastMsg?.role === "agent" &&
		!lastMsg.content &&
		!lastMsg.thinking;

	const isInputDisabled = isStreaming;

	return (
		<div data-testid="agent-chat-panel" className="flex h-full">
			<SessionList
				sessions={sessions}
				activeSessionId={activeSession?.id ?? null}
				onSelect={selectSession}
				onNew={handleNewSession}
			/>
			<div className="flex flex-col flex-1 min-w-0">
				{error && (
					<div className="px-4 py-2 bg-destructive/10 text-destructive text-sm border-b border-destructive/20">
						{error}
					</div>
				)}
				{permissionMode === "plan" && (
					<div
						data-testid="plan-mode-indicator"
						className="px-4 py-1 bg-blue-500/10 text-blue-500 text-xs border-b border-blue-500/20"
					>
						Plan Mode
					</div>
				)}
				{isStreaming && (
					<div
						data-testid="agent-state-indicator"
						className="px-4 py-1 bg-muted text-muted-foreground text-xs border-b"
					>
						{isWaiting ? "Waiting..." : "Running..."}
					</div>
				)}
				<div ref={scrollRef} className="flex-1 overflow-y-auto">
					{activeSession ? (
						<div className="py-2">
							{activeSession.messages.map((msg, idx) => {
								const isLastAgent =
									idx === activeSession.messages.length - 1 &&
									msg.role === "agent";
								const isLastAgentStreaming = isStreaming && isLastAgent;
								const showThinking = isLastAgent && !!msg.thinking;
								const showActivities =
									msg.role === "agent" &&
									msg.activities &&
									msg.activities.length > 0;
								const showMessage = !isLastAgentStreaming || !!msg.content;

								return (
									<div key={msg.id}>
										{showThinking && (
											<ThinkingIndicator
												content={msg.thinking}
												isStreaming={isLastAgentStreaming && !msg.content}
											/>
										)}
										{showActivities && msg.activities && (
											<ActivityLog
												activities={msg.activities}
												isStreaming={isLastAgentStreaming}
											/>
										)}
										{showMessage && (
											<StreamMessage
												message={msg}
												isStreaming={isLastAgentStreaming}
											/>
										)}
									</div>
								);
							})}
							{showWaitingIndicator && (
								<div data-testid="waiting-indicator" className="px-4 py-3">
									<div className="flex items-center gap-2 text-sm text-muted-foreground">
										<Loader2 className="size-4 animate-spin" />
										<span>Waiting...</span>
									</div>
								</div>
							)}
						</div>
					) : (
						<div className="flex items-center justify-center h-full text-muted-foreground text-sm">
							<p>Start a conversation or select a session from the sidebar.</p>
						</div>
					)}
				</div>
				<div className="border-t">
					{pendingPermission && (
						<PermissionDialog
							request={pendingPermission}
							onAllow={(id) => respondPermission(id, true)}
							onDeny={(id) => respondPermission(id, false)}
							onAnswer={(id, answers) =>
								respondPermission(id, true, {
									...pendingPermission.input,
									answers,
								})
							}
						/>
					)}
					<ModeSelector
						mode={permissionMode}
						onModeChange={setPermissionMode}
						disabled={false}
					/>
				</div>
				<MessageInput
					onSend={sendMessage}
					onInterrupt={interrupt}
					disabled={isInputDisabled}
					isStreaming={isStreaming}
				/>
			</div>
		</div>
	);
}
