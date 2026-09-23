import { useCallback, useEffect, useRef, useState } from "react";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import {
	notifyAgentSessionChanged,
	subscribeAgentSessionChanged,
} from "@/lib/agentSessionEvents";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type {
	AgentSessionItem,
	AgentSessionLaunchAttachment,
} from "@/types/agent-session";
import type { Theme } from "@/types/settings";

type OpenOutcome =
	| "attached"
	| "resumed"
	| "restored"
	| "paused"
	| "indeterminate"
	| "garbage_collected";

type PanelState =
	| "loading"
	| "terminal"
	| "paused"
	| "archived"
	| "indeterminate"
	| "gone";

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
	onRefresh?: () => void;
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
	onRefresh,
	initiallyAttached = false,
}: AgentSessionPanelProps) {
	const agentSessionId = session?.id ?? initialAttachment?.agentSessionId ?? "";
	const worktreePath =
		session?.worktreePath ?? initialAttachment?.worktreePath ?? "";
	const workspaceWorktreePath =
		session?.workspaceWorktreePath ??
		initialAttachment?.workspaceWorktreePath ??
		"";
	const workspaceIdentity =
		session?.workspaceIdentity ?? initialAttachment?.workspaceIdentity ?? "";
	const provider = session?.provider ?? initialAttachment?.provider ?? "";
	const pausedMessage = resumeAction
		? "Provider session is not running. Resume to retry."
		: "Provider session is not running.";
	const [state, setState] = useState<PanelState>(
		initiallyAttached ? "terminal" : "loading",
	);
	const [error, setError] = useState<string | null>(null);
	const [terminalError, setTerminalError] = useState<string | null>(null);
	const [actionPending, setActionPending] = useState(false);
	const openedSessionIdRef = useRef<string | null>(
		initiallyAttached ? agentSessionId : null,
	);

	const applyOutcome = useCallback(
		(outcome: OpenOutcome) => {
			setError(null);
			if (
				outcome === "resumed" ||
				outcome === "restored" ||
				outcome === "garbage_collected"
			) {
				notifyAgentSessionChanged(workspaceWorktreePath);
			}
			if (outcome === "resumed" || outcome === "restored") {
				onRefresh?.();
			}
			switch (outcome) {
				case "attached":
				case "resumed":
					setState("terminal");
					return;
				case "restored":
					setState("paused");
					return;
				case "paused":
					onRefresh?.();
					setError(pausedMessage);
					setState("paused");
					return;
				case "indeterminate":
					setState("indeterminate");
					return;
				case "garbage_collected":
					setState("gone");
					return;
			}
		},
		[onRefresh, pausedMessage, workspaceWorktreePath],
	);

	const runLifecycleOperation = useCallback(
		async (command: "open_agent_session" | "restore_agent_session") => {
			if (!session) return;
			setState("loading");
			setError(null);
			try {
				const outcome = await invoke(command, {
					agentSessionId: session.id,
					rows: 24,
					cols: 80,
					callerRequestId: operationId(command),
				});
				applyOutcome(outcome);
			} catch (cause) {
				setError(getErrorMessage(cause));
				setState(
					command === "restore_agent_session" ||
						(command === "open_agent_session" && session.operations.canRestore)
						? "archived"
						: "indeterminate",
				);
			}
		},
		[applyOutcome, session],
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
			setState("gone");
			notifyAgentSessionChanged(session.workspaceWorktreePath);
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

	useEffect(() => {
		if (!session) return;
		if (session.lifecycle === "archived") {
			setState("archived");
		} else if (session.lifecycle === "paused") {
			setState("paused");
		}
	}, [session]);

	if (state === "terminal") {
		return (
			<div className="flex h-full flex-col bg-background">
				{terminalError && (
					<div
						role="alert"
						className="shrink-0 px-3 py-2 text-sm text-destructive"
					>
						{terminalError}
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
			{state === "indeterminate" && (
				<>
					<div className="text-muted-foreground">
						Terminal state is temporarily unavailable.
					</div>
					<Button
						type="button"
						onClick={() => void runLifecycleOperation("open_agent_session")}
					>
						Retry
					</Button>
				</>
			)}
			{state === "gone" && (
				<div className="text-muted-foreground">
					AgentSession is no longer available.
				</div>
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
	const [session, setSession] = useState<AgentSessionItem | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [unavailable, setUnavailable] = useState(false);
	const [attempt, setAttempt] = useState(0);
	const refresh = useCallback(() => setAttempt((value) => value + 1), []);

	useEffect(() => {
		if (launchAttachment) onInitialSessionConsumed?.(agentSessionId);
	}, [agentSessionId, launchAttachment, onInitialSessionConsumed]);

	useEffect(
		() =>
			subscribeAgentSessionChanged(({ worktreePath }) => {
				if (
					!worktreePath ||
					worktreePath ===
						(session?.workspaceWorktreePath ??
							launchAttachment?.workspaceWorktreePath)
				) {
					refresh();
				}
			}),
		[
			launchAttachment?.workspaceWorktreePath,
			refresh,
			session?.workspaceWorktreePath,
		],
	);

	useEffect(() => {
		void attempt;
		let active = true;
		setError(null);
		setUnavailable(false);
		void invoke("get_agent_session", {
			agentSessionId,
		})
			.then((result) => {
				if (!active) return;
				if (!result) {
					setSession(null);
					setUnavailable(true);
					return;
				}
				setSession(result);
			})
			.catch((cause) => {
				if (active) {
					setError(getErrorMessage(cause));
				}
			});
		return () => {
			active = false;
		};
	}, [agentSessionId, attempt]);

	if (
		!unavailable &&
		(session?.id === agentSessionId || launchAttachment != null)
	) {
		return (
			<AgentSessionPanel
				resumeAction={resumeAction}
				session={session?.id === agentSessionId ? session : null}
				initialAttachment={launchAttachment}
				theme={theme}
				onRefresh={refresh}
				initiallyAttached={launchAttachment != null}
			/>
		);
	}

	return (
		<div className="flex h-full flex-col items-center justify-center gap-3 bg-background p-4 text-sm">
			{unavailable ? (
				<div className="text-muted-foreground">
					AgentSession is no longer available.
				</div>
			) : error ? (
				<>
					<div role="alert" className="text-destructive">
						{error}
					</div>
					<Button
						type="button"
						onClick={() => setAttempt((value) => value + 1)}
					>
						Retry
					</Button>
				</>
			) : (
				<div>Loading AgentSession...</div>
			)}
		</div>
	);
}
