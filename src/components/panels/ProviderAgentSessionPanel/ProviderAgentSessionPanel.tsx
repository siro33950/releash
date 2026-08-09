import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { Button } from "@/components/ui/button";
import {
	notifyProviderAgentSessionChanged,
	subscribeProviderAgentSessionChanged,
} from "@/lib/providerAgentSessionEvents";
import type {
	ProviderAgentSessionItem,
	ProviderAgentSessionLaunchAttachment,
} from "@/types/provider-agent-session";
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

interface ProviderAgentSessionPanelProps {
	session: ProviderAgentSessionItem | null;
	initialAttachment?: ProviderAgentSessionLaunchAttachment | null;
	theme?: Theme;
	onUnavailable?: () => void;
	onRefresh?: () => void;
	initiallyAttached?: boolean;
}

interface ProviderAgentSessionRouteProps {
	agentSessionId: string;
	theme?: Theme;
	onUnavailable?: () => void;
	initialAttachment?: ProviderAgentSessionLaunchAttachment;
	onInitialSessionConsumed?: (agentSessionId: string) => void;
}

function operationId(prefix: string): string {
	return `${prefix}.${crypto.randomUUID()}`;
}

export function ProviderAgentSessionPanel({
	session,
	initialAttachment,
	theme,
	onUnavailable,
	onRefresh,
	initiallyAttached = false,
}: ProviderAgentSessionPanelProps) {
	const agentSessionId = session?.id ?? initialAttachment?.agentSessionId ?? "";
	const worktreePath =
		session?.worktreePath ?? initialAttachment?.worktreePath ?? "";
	const workspaceIdentity =
		session?.workspaceIdentity ?? initialAttachment?.workspaceIdentity ?? "";
	const provider = session?.provider ?? initialAttachment?.provider ?? "";
	const [state, setState] = useState<PanelState>(
		initiallyAttached ? "terminal" : "loading",
	);
	const [error, setError] = useState<string | null>(null);
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
				notifyProviderAgentSessionChanged(worktreePath);
			}
			if (outcome === "resumed" || outcome === "restored") {
				onRefresh?.();
			}
			switch (outcome) {
				case "attached":
				case "resumed":
				case "restored":
					setState("terminal");
					return;
				case "paused":
					setError("Provider session is not running. Resume to retry.");
					setState("paused");
					return;
				case "indeterminate":
					setState("indeterminate");
					return;
				case "garbage_collected":
					setState("gone");
					onUnavailable?.();
			}
		},
		[onRefresh, onUnavailable, worktreePath],
	);

	const runLifecycleOperation = useCallback(
		async (
			command:
				| "open_provider_agent_session"
				| "resume_provider_agent_session"
				| "restore_provider_agent_session",
		) => {
			if (!session) return;
			setState("loading");
			setError(null);
			try {
				const outcome = await invoke<OpenOutcome>(command, {
					agentSessionId: session.id,
					rows: 24,
					cols: 80,
					callerRequestId: operationId(command),
				});
				applyOutcome(outcome);
			} catch (cause) {
				setError(cause instanceof Error ? cause.message : String(cause));
				setState(
					command === "restore_provider_agent_session" ||
						(command === "open_provider_agent_session" &&
							session.operations.canRestore)
						? "archived"
						: command === "resume_provider_agent_session"
							? "paused"
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
			await invoke("delete_provider_agent_session", {
				agentSessionId: session.id,
				callerRequestId: operationId("delete_provider_agent_session"),
			});
			setState("gone");
			notifyProviderAgentSessionChanged(session.worktreePath);
			onUnavailable?.();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setActionPending(false);
		}
	}, [onUnavailable, session]);

	useEffect(() => {
		if (!session) return;
		if (openedSessionIdRef.current === session.id) return;
		openedSessionIdRef.current = session.id;
		void runLifecycleOperation("open_provider_agent_session");
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
			<TerminalPanel
				cwd={worktreePath}
				theme={theme}
				owner={{
					kind: "session",
					workspacePath: workspaceIdentity,
					sessionId: agentSessionId,
				}}
				label={`${provider} AgentSession`}
				initialization="attach-existing"
				autoFocus
			/>
		);
	}

	return (
		<div className="flex h-full flex-col items-center justify-center gap-3 bg-background p-4 text-sm">
			{error && (
				<div role="alert" className="text-destructive">
					{error}
				</div>
			)}
			{state === "loading" && <div>Opening AgentSession...</div>}
			{state === "paused" && (
				<>
					<div className="text-muted-foreground">AgentSession is paused.</div>
					<Button
						type="button"
						onClick={() =>
							void runLifecycleOperation("resume_provider_agent_session")
						}
					>
						Resume
					</Button>
				</>
			)}
			{state === "archived" && (
				<>
					<div className="text-muted-foreground">AgentSession is archived.</div>
					<Button
						type="button"
						disabled={actionPending}
						onClick={() =>
							void runLifecycleOperation("restore_provider_agent_session")
						}
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
						onClick={() =>
							void runLifecycleOperation("open_provider_agent_session")
						}
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

export function ProviderAgentSessionRoute({
	agentSessionId,
	theme,
	onUnavailable,
	initialAttachment,
	onInitialSessionConsumed,
}: ProviderAgentSessionRouteProps) {
	const [launchAttachment] =
		useState<ProviderAgentSessionLaunchAttachment | null>(
			initialAttachment?.agentSessionId === agentSessionId
				? initialAttachment
				: null,
		);
	const [session, setSession] = useState<ProviderAgentSessionItem | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [unavailable, setUnavailable] = useState(false);
	const [attempt, setAttempt] = useState(0);
	const refresh = useCallback(() => setAttempt((value) => value + 1), []);

	useEffect(() => {
		if (launchAttachment) onInitialSessionConsumed?.(agentSessionId);
	}, [agentSessionId, launchAttachment, onInitialSessionConsumed]);

	useEffect(
		() =>
			subscribeProviderAgentSessionChanged(({ worktreePath }) => {
				if (
					!worktreePath ||
					worktreePath ===
						(session?.worktreePath ?? launchAttachment?.worktreePath)
				) {
					refresh();
				}
			}),
		[launchAttachment?.worktreePath, refresh, session?.worktreePath],
	);

	useEffect(() => {
		void attempt;
		let active = true;
		setError(null);
		setUnavailable(false);
		void invoke<ProviderAgentSessionItem | null>("get_provider_agent_session", {
			agentSessionId,
		})
			.then((result) => {
				if (!active) return;
				if (!result) {
					setSession(null);
					setUnavailable(true);
					onUnavailable?.();
					return;
				}
				setSession(result);
			})
			.catch((cause) => {
				if (active) {
					setError(cause instanceof Error ? cause.message : String(cause));
				}
			});
		return () => {
			active = false;
		};
	}, [agentSessionId, attempt, onUnavailable]);

	if (
		!unavailable &&
		(session?.id === agentSessionId || launchAttachment != null)
	) {
		return (
			<ProviderAgentSessionPanel
				session={session?.id === agentSessionId ? session : null}
				initialAttachment={launchAttachment}
				theme={theme}
				onUnavailable={onUnavailable}
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
