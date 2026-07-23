import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UseAgentChatResult } from "@/hooks/useAgentChat";
import type { SessionFeedbackEntry } from "@/hooks/useSessionStore";
import type {
	AgentStallObservation,
	ChatSession,
	PermissionMode,
	PermissionRequest,
	PlanMode,
	SessionNotice,
} from "@/types/session";

const mocks = vi.hoisted(() => ({
	useAgentChatContext: vi.fn(),
	loadSession: vi.fn().mockResolvedValue(null),
	loadOlderMessages: vi.fn().mockResolvedValue(undefined),
	evictOlderMessages: vi.fn().mockResolvedValue(undefined),
	registerViewableSession: vi.fn(() => vi.fn()),
	sendMessage: vi.fn().mockResolvedValue(true),
	interrupt: vi.fn(),
	cancelQueuedTurn: vi.fn().mockResolvedValue(undefined),
	resumeQueue: vi.fn().mockResolvedValue(undefined),
	setPermissionMode: vi.fn(),
	setPlanMode: vi.fn(),
	setModel: vi.fn(),
	respondPermission: vi.fn(),
	closeSession: vi.fn().mockResolvedValue(undefined),
	archiveSession: vi.fn().mockResolvedValue(undefined),
	archiveOpenSession: vi.fn().mockResolvedValue(undefined),
	dismissSessionError: vi.fn(),
	dismissFeedback: vi.fn().mockResolvedValue(undefined),
	retryFeedback: vi.fn().mockResolvedValue(undefined),
	loadNextFeedback: vi.fn().mockResolvedValue(undefined),
	sessionFeedback: {
		entries: [] as SessionFeedbackEntry[],
		hasMore: false,
	},
}));

vi.mock("@/contexts/AgentChatContext", () => ({
	useAgentChatContext: () => mocks.useAgentChatContext(),
}));

vi.mock("@/hooks/useSessionFeedback", () => ({
	useSessionFeedback: () => ({
		entries: mocks.sessionFeedback.entries,
		hasMore: mocks.sessionFeedback.hasMore,
		dismiss: mocks.dismissFeedback,
		retry: mocks.retryFeedback,
		loadNextPage: mocks.loadNextFeedback,
		refresh: vi.fn(),
	}),
}));

vi.mock("./ChatSessionView", () => ({
	ChatSessionView: ({
		session,
		permissionMode,
		planMode,
		pendingPermission,
		onRespondPermission,
		onSend,
		onModelChange,
		stallObservation,
		notice,
		error,
		onDismissError,
		queuePaused,
		onResumeQueue,
	}: {
		session: ChatSession;
		permissionMode: PermissionMode;
		planMode: PlanMode;
		pendingPermission: PermissionRequest | null;
		onRespondPermission: (
			requestId: string,
			allow: boolean,
			updatedInput?: Record<string, unknown>,
		) => void;
		onSend: (content: string) => Promise<boolean>;
		onModelChange: (modelId: string) => void;
		stallObservation?: AgentStallObservation | null;
		notice?: SessionNotice | null;
		error: string | null;
		onDismissError: () => void;
		queuePaused: boolean;
		onResumeQueue: () => Promise<void>;
	}) => (
		<div data-testid={`chat-${session.id}`}>
			<span data-testid={`permission-${session.id}`}>{permissionMode}</span>
			<span data-testid={`plan-${session.id}`}>{String(planMode)}</span>
			<span data-testid={`pending-${session.id}`}>
				{pendingPermission?.id ?? "none"}
			</span>
			{pendingPermission && (
				<button
					type="button"
					data-testid={`respond-${session.id}`}
					onClick={() =>
						onRespondPermission(pendingPermission.id, true, {
							answer: "approved",
						})
					}
				>
					Respond
				</button>
			)}
			<button type="button" onClick={() => void onSend("hello")}>
				Send message
			</button>
			<button type="button" onClick={() => onModelChange("codex:gpt-5")}>
				Select Codex model
			</button>
			<span data-testid={`stall-${session.id}`}>
				{stallObservation
					? `${stallObservation.turnPhase}:${stallObservation.idleSecs}`
					: "none"}
			</span>
			<span data-testid={`notice-${session.id}`}>
				{notice?.message ?? "none"}
			</span>
			{error && (
				<div data-testid={`error-${session.id}`}>
					{error}
					<button type="button" onClick={onDismissError}>
						Dismiss error
					</button>
				</div>
			)}
			<span data-testid={`queue-paused-${session.id}`}>
				{String(queuePaused)}
			</span>
			<button type="button" onClick={() => void onResumeQueue()}>
				Resume queue
			</button>
		</div>
	),
}));

const { BoundSessionChat } = await import("./BoundSessionChat");

function makeSession(
	id: string,
	permissionMode: PermissionMode,
	planMode: PlanMode,
): ChatSession {
	return {
		id,
		worktreePath: "/repo",
		messages: [],
		state: "active",
		createdAt: 1000,
		updatedAt: 1000,
		permissionMode,
		planMode,
	};
}

function makePendingPermission(id: string): PermissionRequest {
	return {
		id,
		toolName: "Bash",
		kind: "tool_approval",
		input: { command: "echo hi" },
		title: "Run command",
	};
}

function setContext(
	sessionsById: Record<string, ChatSession>,
	pendingPermissions: Record<string, PermissionRequest | null> = {},
	stallObservations: Record<string, AgentStallObservation | null> = {},
	notices: Record<string, SessionNotice | null> = {},
	sessionErrors: Record<string, string | null> = {},
	queuePausedBySession: Record<string, boolean> = {},
) {
	const context: UseAgentChatResult = {
		sessions: [],
		orderedSessions: [],
		closedSessions: [],
		activeSession: null,
		isStreaming: false,
		activityStatus: null,
		permissionMode: "edit",
		planMode: false,
		sessionAgentStates: new Map(),
		getSessionById: (sessionId: string | null | undefined) =>
			sessionId ? (sessionsById[sessionId] ?? null) : null,
		loadSession: mocks.loadSession,
		loadOlderMessages: mocks.loadOlderMessages,
		evictOlderMessages: mocks.evictOlderMessages,
		registerViewableSession: mocks.registerViewableSession,
		getSessionTurnPhase: vi.fn().mockReturnValue("idle"),
		getSessionInterrupting: vi.fn().mockReturnValue(false),
		getSessionPermissionMode: (sessionId: string) =>
			sessionsById[sessionId]?.permissionMode ?? "edit",
		getSessionPlanMode: (sessionId: string) =>
			sessionsById[sessionId]?.planMode ?? false,
		getSessionSelectedModel: vi.fn().mockReturnValue(null),
		getSessionCanChangeBackend: vi.fn().mockReturnValue(false),
		getSessionPendingPermission: (sessionId: string) =>
			pendingPermissions[sessionId] ?? null,
		getSessionPendingQueue: vi.fn().mockReturnValue([]),
		getSessionQueuePaused: (sessionId: string) =>
			queuePausedBySession[sessionId] ?? false,
		getSessionStallObservation: (sessionId: string) =>
			stallObservations[sessionId] ?? null,
		getSessionNotice: (sessionId: string) => notices[sessionId] ?? null,
		getSessionLatestTokenUsage: vi.fn().mockReturnValue(null),
		getSessionRuntimeSlashCommands: vi.fn().mockReturnValue([]),
		getSessionError: (sessionId: string) => sessionErrors[sessionId] ?? null,
		dismissSessionError: mocks.dismissSessionError,
		availableModels: [],
		availableModelsByBackend: {},
		selectedModel: null,
		backends: [],
		selectedBackendId: null,
		sendMessage: mocks.sendMessage,
		interrupt: mocks.interrupt,
		selectSession: vi.fn().mockResolvedValue(undefined),
		refreshSessions: vi.fn().mockResolvedValue(undefined),
		refreshClosedSessions: vi.fn().mockResolvedValue(undefined),
		closeSession: mocks.closeSession,
		archiveSession: mocks.archiveSession,
		archiveOpenSession: mocks.archiveOpenSession,
		restoreSession: vi.fn().mockResolvedValue(undefined),
		forkSession: vi.fn().mockResolvedValue(undefined),
		setSessionTitle: vi.fn().mockResolvedValue("title"),
		createNewSession: vi.fn().mockResolvedValue(null),
		createNewWorkspaceSession: vi.fn().mockResolvedValue("session"),
		reorderSessions: vi.fn(),
		cancelQueuedTurn: mocks.cancelQueuedTurn,
		resumeQueue: mocks.resumeQueue,
		setPermissionMode: mocks.setPermissionMode,
		setPlanMode: mocks.setPlanMode,
		setModel: mocks.setModel,
		setBackend: vi.fn(),
		respondPermission: mocks.respondPermission,
	};
	mocks.useAgentChatContext.mockReturnValue(context);
}

describe("BoundSessionChat", () => {
	beforeEach(() => {
		for (const mock of Object.values(mocks)) {
			if (typeof mock === "function" && "mockClear" in mock) {
				mock.mockClear();
			}
		}
		mocks.sessionFeedback.entries = [];
		mocks.sessionFeedback.hasMore = false;
	});

	it("close_quit_chat_panel_close_is_view_only", () => {
		const cleanupViewRegistration = vi.fn();
		mocks.registerViewableSession.mockReturnValueOnce(cleanupViewRegistration);
		const session = makeSession("session-active", "ask", false);
		session.messages = [
			{
				id: "agent-1",
				role: "agent",
				parts: [{ type: "text", content: "durable and retained" }],
				timestamp: 1,
			},
		];
		const permission = makePendingPermission("permission-retained");
		setContext(
			{ [session.id]: session },
			{ [session.id]: permission },
			{},
			{},
			{},
			{ [session.id]: true },
		);

		const { unmount } = render(
			<BoundSessionChat
				sessionId={session.id}
				worktreePath="/repo"
				skipInitialLoad
			/>,
		);
		expect(screen.getByTestId(`chat-${session.id}`)).toBeInTheDocument();

		unmount();

		expect(cleanupViewRegistration).toHaveBeenCalledOnce();
		expect(session.state).toBe("active");
		expect(session.messages[0].parts).toEqual([
			{ type: "text", content: "durable and retained" },
		]);
		expect(permission.id).toBe("permission-retained");
		expect(mocks.closeSession).not.toHaveBeenCalled();
		expect(mocks.archiveSession).not.toHaveBeenCalled();
		expect(mocks.archiveOpenSession).not.toHaveBeenCalled();
		expect(mocks.interrupt).not.toHaveBeenCalled();
		expect(mocks.respondPermission).not.toHaveBeenCalled();
		expect(mocks.resumeQueue).not.toHaveBeenCalled();
	});

	it("passes each pane its own session permission and plan mode", () => {
		setContext({
			"session-a": makeSession("session-a", "ask", false),
			"session-b": makeSession("session-b", "full", true),
		});

		render(
			<div>
				<BoundSessionChat
					sessionId="session-a"
					worktreePath="/repo"
					skipInitialLoad
				/>
				<BoundSessionChat
					sessionId="session-b"
					worktreePath="/repo"
					skipInitialLoad
				/>
			</div>,
		);

		expect(screen.getByTestId("permission-session-a")).toHaveTextContent("ask");
		expect(screen.getByTestId("plan-session-a")).toHaveTextContent("false");
		expect(screen.getByTestId("permission-session-b")).toHaveTextContent(
			"full",
		);
		expect(screen.getByTestId("plan-session-b")).toHaveTextContent("true");
	});

	it("binds model selection to the session displayed by the pane", () => {
		setContext({
			"session-a": makeSession("session-a", "ask", false),
		});

		render(
			<BoundSessionChat
				sessionId="session-a"
				worktreePath="/repo"
				skipInitialLoad
			/>,
		);
		fireEvent.click(screen.getByText("Select Codex model"));

		expect(mocks.setModel).toHaveBeenCalledWith("session-a", "codex:gpt-5");
	});

	it("shows each session error only in its source pane", () => {
		setContext(
			{
				"session-a": makeSession("session-a", "ask", false),
				"session-b": makeSession("session-b", "full", true),
			},
			{},
			{},
			{},
			{ "session-b": "send failed in B" },
		);

		render(
			<div>
				<BoundSessionChat
					sessionId="session-a"
					worktreePath="/repo"
					skipInitialLoad
				/>
				<BoundSessionChat
					sessionId="session-b"
					worktreePath="/repo"
					skipInitialLoad
				/>
			</div>,
		);

		expect(screen.queryByTestId("error-session-a")).not.toBeInTheDocument();
		expect(screen.getByTestId("error-session-b")).toHaveTextContent(
			"send failed in B",
		);
	});

	it("keeps session A error visible when session B updates and dismisses only A", () => {
		const sessions = {
			"session-a": makeSession("session-a", "ask", false),
			"session-b": makeSession("session-b", "full", true),
		};
		const errors = {
			"session-a": "send failed in A",
			"session-b": "load failed in B",
		};
		setContext(sessions, {}, {}, {}, errors);

		const { rerender } = render(
			<div>
				<BoundSessionChat
					sessionId="session-a"
					worktreePath="/repo"
					skipInitialLoad
				/>
				<BoundSessionChat
					sessionId="session-b"
					worktreePath="/repo"
					skipInitialLoad
				/>
			</div>,
		);

		sessions["session-b"] = {
			...sessions["session-b"],
			state: "active",
			updatedAt: 2000,
		};
		setContext(sessions, {}, {}, {}, errors);
		rerender(
			<div>
				<BoundSessionChat
					sessionId="session-a"
					worktreePath="/repo"
					skipInitialLoad
				/>
				<BoundSessionChat
					sessionId="session-b"
					worktreePath="/repo"
					skipInitialLoad
				/>
			</div>,
		);

		expect(screen.getByTestId("error-session-a")).toHaveTextContent(
			"send failed in A",
		);
		expect(screen.getByTestId("error-session-b")).toHaveTextContent(
			"load failed in B",
		);

		fireEvent.click(
			screen.getByTestId("error-session-a").querySelector("button") as Element,
		);
		expect(mocks.dismissSessionError).toHaveBeenCalledWith("session-a");
		expect(mocks.dismissSessionError).not.toHaveBeenCalledWith("session-b");
	});

	it("passes each selected Node session its own pending permission and response handler", () => {
		const permissionA = makePendingPermission("perm-a");
		const permissionB = makePendingPermission("perm-b");
		setContext(
			{
				"session-a": makeSession("session-a", "ask", false),
				"session-b": makeSession("session-b", "full", true),
			},
			{
				"session-a": permissionA,
				"session-b": permissionB,
			},
		);

		render(
			<div>
				<BoundSessionChat
					sessionId="session-a"
					worktreePath="/repo"
					skipInitialLoad
				/>
				<BoundSessionChat
					sessionId="session-b"
					worktreePath="/repo"
					skipInitialLoad
				/>
			</div>,
		);

		expect(screen.getByTestId("pending-session-a")).toHaveTextContent("perm-a");
		expect(screen.getByTestId("pending-session-b")).toHaveTextContent("perm-b");

		fireEvent.click(screen.getByTestId("respond-session-b"));

		expect(mocks.respondPermission).toHaveBeenCalledWith(
			"session-b",
			"perm-b",
			true,
			{ answer: "approved" },
		);
		expect(mocks.respondPermission).not.toHaveBeenCalledWith(
			"session-a",
			"perm-b",
			true,
			{ answer: "approved" },
		);
	});

	it("passes the selected session stall observation to ChatSessionView", () => {
		setContext(
			{
				"session-a": makeSession("session-a", "edit", false),
			},
			{},
			{
				"session-a": {
					turnPhase: "streaming",
					idleSecs: 181,
					signalCount: 2,
					capReached: false,
				},
			},
		);

		render(
			<BoundSessionChat
				sessionId="session-a"
				worktreePath="/repo"
				skipInitialLoad
			/>,
		);

		expect(screen.getByTestId("stall-session-a")).toHaveTextContent(
			"streaming:181",
		);
	});

	it("passes only the selected session notice to ChatSessionView", () => {
		setContext(
			{
				"session-a": makeSession("session-a", "edit", false),
				"session-b": makeSession("session-b", "edit", false),
			},
			{},
			{},
			{
				"session-a": {
					sessionId: "session-a",
					kind: "persist_failure",
					message: "Session A notice",
					createdAt: 2_000,
				},
				"session-b": {
					sessionId: "session-b",
					kind: "event_log_recovered",
					message: "Session B notice",
					createdAt: 3_000,
				},
			},
		);

		render(
			<BoundSessionChat
				sessionId="session-a"
				worktreePath="/repo"
				skipInitialLoad
			/>,
		);

		expect(screen.getByTestId("notice-session-a")).toHaveTextContent(
			"Session A notice",
		);
		expect(screen.getByTestId("notice-session-a")).not.toHaveTextContent(
			"Session B notice",
		);
	});

	it("shows a paused queue and delegates resume for the selected session", () => {
		setContext(
			{
				"session-a": makeSession("session-a", "edit", false),
			},
			{},
			{},
			{},
			{},
			{ "session-a": true },
		);

		render(
			<BoundSessionChat
				sessionId="session-a"
				worktreePath="/repo"
				skipInitialLoad
			/>,
		);

		expect(screen.getByTestId("queue-paused-session-a")).toHaveTextContent(
			"true",
		);
		fireEvent.click(screen.getByRole("button", { name: "Resume queue" }));
		expect(mocks.resumeQueue).toHaveBeenCalledWith("session-a");
	});

	it("loads and registers a selected Workflow Session for live updates and input", async () => {
		const cleanup = vi.fn();
		mocks.registerViewableSession.mockReturnValue(cleanup);
		setContext({
			"workflow-session": makeSession("workflow-session", "ask", false),
		});

		const { unmount } = render(
			<BoundSessionChat sessionId="workflow-session" worktreePath="/repo" />,
		);

		expect(mocks.loadSession).toHaveBeenCalledWith("workflow-session");
		expect(mocks.registerViewableSession).toHaveBeenCalledWith(
			"workflow-session",
		);
		fireEvent.click(screen.getByRole("button", { name: "Send message" }));
		expect(mocks.sendMessage).toHaveBeenCalledWith(
			"workflow-session",
			"hello",
			undefined,
			undefined,
		);

		unmount();
		expect(cleanup).toHaveBeenCalled();
	});

	it("shows loading and then renders the Workflow Session returned by the initial load", async () => {
		const sessions: Record<string, ChatSession> = {};
		let resolveLoad: (session: ChatSession) => void = () => {};
		mocks.loadSession.mockImplementationOnce((sessionId: string) => {
			return new Promise<ChatSession>((resolve) => {
				resolveLoad = (loaded) => {
					sessions[sessionId] = loaded;
					resolve(loaded);
				};
			});
		});
		setContext(sessions);

		render(
			<BoundSessionChat sessionId="workflow-session" worktreePath="/repo" />,
		);

		expect(screen.getByRole("status")).toHaveTextContent("Loading session...");
		await act(async () => {
			resolveLoad(makeSession("workflow-session", "ask", false));
		});
		expect(await screen.findByTestId("chat-workflow-session")).toBeVisible();
	});

	it("reports an unavailable Workflow Session instead of leaving the center blank", async () => {
		mocks.loadSession.mockResolvedValueOnce(null);
		setContext({});

		render(
			<BoundSessionChat sessionId="missing-session" worktreePath="/repo" />,
		);

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"Session unavailable.",
		);
	});

	it("keeps canonical feedback visible and controllable when session loading fails", async () => {
		const feedback: SessionFeedbackEntry = {
			feedback_id: "feedback-load-1",
			attempt_id: "load-attempt-1",
			session_id: "missing-session",
			operation: "load_session",
			revision: "1",
			actions: ["dismiss"],
			action_identities: [
				{
					action: "dismiss",
					action_id: "dismiss-load-1",
					origin_revision: "1",
				},
			],
			failure: {
				kind: "persist_failure",
				retryable: true,
				label: "The session could not be loaded.",
				detail: "Retry loading the session or dismiss this feedback.",
				correlation_id: "load-failure-1",
			},
		};
		mocks.sessionFeedback.entries = [feedback];
		mocks.sessionFeedback.hasMore = true;
		mocks.loadSession.mockRejectedValueOnce(new Error("unavailable"));
		setContext(
			{},
			{},
			{},
			{},
			{
				"missing-session": "legacy duplicate",
			},
		);

		render(
			<BoundSessionChat sessionId="missing-session" worktreePath="/repo" />,
		);

		expect(
			await screen.findByText("The session could not be loaded."),
		).toBeVisible();
		expect(screen.getByText("Session unavailable.")).toBeVisible();
		expect(screen.queryByText("legacy duplicate")).not.toBeInTheDocument();
		fireEvent.click(screen.getByRole("button", { name: "Dismiss feedback" }));
		expect(mocks.dismissFeedback).toHaveBeenCalledWith(feedback);
		fireEvent.click(screen.getByRole("button", { name: "Load more feedback" }));
		expect(mocks.loadNextFeedback).toHaveBeenCalledTimes(1);
	});

	it("shows and dismisses a load failure notice before the session is hydrated", async () => {
		const sessionErrors: Record<string, string> = {};
		mocks.loadSession.mockImplementationOnce(async (sessionId: string) => {
			sessionErrors[sessionId] = "セッション読み込みに失敗: unavailable";
			throw new Error("unavailable");
		});
		setContext({}, {}, {}, {}, sessionErrors);

		render(
			<BoundSessionChat sessionId="missing-session" worktreePath="/repo" />,
		);

		expect(
			await screen.findByText("セッション読み込みに失敗: unavailable"),
		).toBeVisible();
		fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
		expect(mocks.dismissSessionError).toHaveBeenCalledWith("missing-session");
	});
});
