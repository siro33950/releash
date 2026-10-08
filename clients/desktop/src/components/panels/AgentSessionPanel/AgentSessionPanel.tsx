import { useCallback, useEffect, useRef, useState } from "react";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type {
	AgentSessionItem,
	AgentSessionLaunchAttachment,
} from "@/types/agent-session";
import type { Theme } from "@/types/settings";

export interface SessionResumeAction {
	pending: boolean;
	error: string | null;
	onResume: () => void;
}

export function SessionResumeButton({
	action,
}: {
	action: SessionResumeAction;
}) {
	return (
		<>
			{action.error && (
				<div role="alert" className="max-w-md break-words text-destructive">
					{action.error}
				</div>
			)}
			<Button type="button" disabled={action.pending} onClick={action.onResume}>
				{action.pending ? "Resuming..." : "Resume"}
			</Button>
		</>
	);
}

interface AgentSessionPanelProps {
	resumeAction?: SessionResumeAction | null;
	session: AgentSessionItem | null;
	initialAttachment?: AgentSessionLaunchAttachment | null;
	theme?: Theme;

	initiallyAttached?: boolean;
}

interface AgentSessionRouteProps {
	resumeAction?: SessionResumeAction | null;
	agentSessionId: string;
	theme?: Theme;
	initialAttachment?: AgentSessionLaunchAttachment;
	onInitialSessionConsumed?: (agentSessionId: string) => void;
}

function operationId(prefix: string): string {
	return `${prefix}.${crypto.randomUUID()}`;
}

export function AgentSessionPanel({
	resumeAction = null,
	session,
	initialAttachment,
	theme,

	initiallyAttached = false,
}: AgentSessionPanelProps) {
	const agentSessionId = session?.id ?? initialAttachment?.agentSessionId ?? "";
	const worktreePath =
		session?.worktreePath ?? initialAttachment?.worktreePath ?? "";
	const workspaceIdentity =
		session?.workspaceIdentity ?? initialAttachment?.workspaceIdentity ?? "";
	const provider = session?.provider ?? initialAttachment?.provider ?? "";
	const pausedMessage = resumeAction
		? "Provider session is not running. Resume to retry."
		: "Provider session is not running.";
	const state =
		session?.lifecycle === "open"
			? session.terminalPresence === "unknown"
				? "indeterminate"
				: session.terminalPresence === "absent"
					? "paused"
					: "terminal"
			: (session?.lifecycle ?? "loading");
	const [error, setError] = useState<string | null>(null);
	const [terminalError, setTerminalError] = useState<string | null>(null);
	const [actionPending, setActionPending] = useState(false);
	const openedSessionIdRef = useRef<string | null>(
		initiallyAttached ? agentSessionId : null,
	);

	const runLifecycleOperation = useCallback(
		async (command: "open_agent_session" | "restore_agent_session") => {
			if (!session) return;
			setError(null);
			try {
				await invoke(command, {
					agentSessionId: session.id,
					rows: 24,
					cols: 80,
					callerRequestId: operationId(command),
				});
			} catch (cause) {
				setError(getErrorMessage(cause));
			}
		},
		[session],
	);

	const remove = useCallback(async () => {
		if (!session) return;
		setActionPending(true);
		setError(null);
		try {
			await invoke("delete_agent_session", {
				agentSessionId: session.id,
				callerRequestId: operationId("delete_agent_session"),
			});
		} catch (cause) {
			setError(getErrorMessage(cause));
		} finally {
			setActionPending(false);
		}
	}, [session]);

	useEffect(() => {
		if (!session) return;
		if (openedSessionIdRef.current === session.id) return;
		openedSessionIdRef.current = session.id;
		void runLifecycleOperation("open_agent_session");
	}, [runLifecycleOperation, session]);

	if (state === "terminal") {
		return (
			<div className="flex h-full flex-col bg-background">
				{(error || terminalError) && (
					<div
						role="alert"
						className="shrink-0 px-3 py-2 text-sm text-destructive"
					>
						{error ?? terminalError}
					</div>
				)}
				<div className="min-h-0 flex-1">
					<TerminalPanel
						cwd={worktreePath}
						theme={theme}
						owner={{
							kind: "session",
							workspacePath: workspaceIdentity,
							sessionId: agentSessionId,
						}}
						label={`${provider} AgentSession`}
						onTerminalError={setTerminalError}
						initialization="attach-existing"
						autoFocus
					/>
				</div>
			</div>
		);
	}

	return (
		<div className="flex h-full flex-col items-center justify-center gap-3 bg-background p-4 text-sm">
			{error && (
				<div role="alert" className="text-destructive">
					{error}
				</div>
			)}
			{state === "paused" && !error && (
				<div role="alert" className="text-destructive">
					{pausedMessage}
				</div>
			)}
			{state === "loading" && <div>Opening AgentSession...</div>}
			{state === "indeterminate" && (
				<div role="alert">Provider process state is not confirmed.</div>
			)}
			{state === "paused" && (
				<>
					<div className="text-muted-foreground">AgentSession is paused.</div>
					{resumeAction && <SessionResumeButton action={resumeAction} />}
				</>
			)}
			{state === "archived" && (
				<>
					<div className="text-muted-foreground">AgentSession is archived.</div>
					<Button
						type="button"
						disabled={actionPending}
						onClick={() => void runLifecycleOperation("restore_agent_session")}
					>
						Restore
					</Button>
					<Button
						type="button"
						variant="destructive"
						disabled={actionPending}
						onClick={() => void remove()}
					>
						Delete
					</Button>
				</>
			)}
		</div>
	);
}

export function AgentSessionRoute({
	resumeAction = null,
	agentSessionId,
	theme,
	initialAttachment,
	onInitialSessionConsumed,
}: AgentSessionRouteProps) {
	const [launchAttachment] = useState<AgentSessionLaunchAttachment | null>(
		initialAttachment?.agentSessionId === agentSessionId
			? initialAttachment
			: null,
	);
	const subscription = useStateSubscriptionResult({
		kind: "agent-session",
		args: [agentSessionId],
	});
	const session = subscription.value;
	const unavailable = session === null;
	useEffect(() => {
		if (launchAttachment) onInitialSessionConsumed?.(agentSessionId);
	}, [agentSessionId, launchAttachment, onInitialSessionConsumed]);

	if (
		!unavailable &&
		(session?.id === agentSessionId || launchAttachment != null)
	) {
		return (
			<div className="flex h-full flex-col">
				{subscription.error && <div role="alert">{subscription.error}</div>}
				<div className="min-h-0 flex-1">
					<AgentSessionPanel
						resumeAction={resumeAction}
						session={session?.id === agentSessionId ? session : null}
						initialAttachment={launchAttachment}
						theme={theme}
						initiallyAttached={launchAttachment != null}
					/>
				</div>
			</div>
		);
	}

	return (
		<div className="flex h-full flex-col items-center justify-center gap-3 bg-background p-4 text-sm">
			{unavailable ? (
				<>
					<div className="text-muted-foreground">
						AgentSession is no longer available.
					</div>
					{resumeAction && <SessionResumeButton action={resumeAction} />}
				</>
			) : subscription.error ? (
				<div role="alert">{subscription.error}</div>
			) : (
				<div>Loading AgentSession...</div>
			)}
		</div>
	);
}
