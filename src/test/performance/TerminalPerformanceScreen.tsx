import { useEffect, useState } from "react";
import { TerminalPanel } from "@/components/panels/TerminalPanel";
import { installPerformanceCollector } from "./performanceCollector";

interface TerminalPerformanceSessionAttachment {
	agentSessionId: string;
	workspaceIdentity: string;
	worktreePath: string;
	provider: "claude" | "codex";
	launchStartedAt: number;
}

interface TerminalPerformanceSessionDriver {
	mountSession(attachment: TerminalPerformanceSessionAttachment): void;
	clearSession(): void;
}

declare global {
	interface Window {
		__RELEASH_TERMINAL_PERFORMANCE_SESSION_DRIVER__?: TerminalPerformanceSessionDriver;
	}
}

installPerformanceCollector();

const PERFORMANCE_OWNER = {
	kind: "workspace" as const,
	workspacePath: `releash-performance-terminal-${crypto.randomUUID()}`,
};

export function TerminalPerformanceScreen() {
	const [terminalReady, setTerminalReady] = useState(false);
	const [selection, setSelection] = useState("primary");
	const [terminalError, setTerminalError] = useState<string | null>(null);
	const [terminalVisible, setTerminalVisible] = useState(true);
	const [sessionAttachment, setSessionAttachment] = useState<
		TerminalPerformanceSessionAttachment | null | undefined
	>(undefined);
	const [sessionReady, setSessionReady] = useState(false);
	const [sessionError, setSessionError] = useState<string | null>(null);

	useEffect(() => {
		window.__RELEASH_TERMINAL_PERFORMANCE_SESSION_DRIVER__ = {
			mountSession: (attachment) => {
				const state = window.__RELEASH_TERMINAL_PERFORMANCE_STATE__;
				if (!state)
					throw new Error("Terminal performance collector is missing");
				state.launchOrigins[attachment.agentSessionId] =
					attachment.launchStartedAt;
				setSessionReady(false);
				setSessionError(null);
				setSessionAttachment(attachment);
			},
			clearSession: () => {
				setSessionReady(false);
				setSessionError(null);
				setSessionAttachment(null);
			},
		};
		return () => {
			delete window.__RELEASH_TERMINAL_PERFORMANCE_SESSION_DRIVER__;
		};
	}, []);

	return (
		<main className="flex h-screen min-h-0 bg-background text-foreground">
			<aside className="flex w-48 shrink-0 flex-col gap-2 border-r p-2">
				<button
					type="button"
					data-testid="performance-workspace-selection"
					data-selection={selection}
					onClick={() =>
						setSelection((current) =>
							current === "primary" ? "secondary" : "primary",
						)
					}
				>
					{selection}
				</button>
				<button
					type="button"
					data-testid="performance-terminal-visibility"
					data-visible={terminalVisible}
					onClick={() => {
						setTerminalReady(false);
						setTerminalVisible((visible) => !visible);
					}}
				>
					{terminalVisible ? "hide" : "show"}
				</button>
				<div data-testid="performance-terminal-ready">
					{terminalReady ? "ready" : "starting"}
				</div>
				{terminalError ? (
					<div data-testid="performance-terminal-error">{terminalError}</div>
				) : null}
				<div
					data-testid="performance-session-ready"
					data-session-id={sessionAttachment?.agentSessionId ?? ""}
				>
					{sessionError ?? (sessionReady ? "ready" : "idle")}
				</div>
			</aside>
			<section
				className="min-h-0 min-w-0 flex-1"
				data-testid="performance-terminal"
				data-owner-workspace-path={PERFORMANCE_OWNER.workspacePath}
			>
				{sessionAttachment ? (
					<TerminalPanel
						cwd={sessionAttachment.worktreePath}
						owner={{
							kind: "session",
							workspacePath: sessionAttachment.workspaceIdentity,
							sessionId: sessionAttachment.agentSessionId,
						}}
						label={`${sessionAttachment.provider} performance AgentSession`}
						theme="dark"
						initialization="attach-existing"
						autoFocus
						onTerminalReady={() => setSessionReady(true)}
						onTerminalError={setSessionError}
					/>
				) : sessionAttachment === undefined && terminalVisible ? (
					<TerminalPanel
						owner={PERFORMANCE_OWNER}
						label="Performance Terminal"
						theme="dark"
						autoFocus
						onTerminalReady={() => setTerminalReady(true)}
						onTerminalError={setTerminalError}
					/>
				) : null}
			</section>
		</main>
	);
}
