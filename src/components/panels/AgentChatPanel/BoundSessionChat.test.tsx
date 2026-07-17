import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
	AgentStallObservation,
	ChatSession,
	PermissionMode,
	PermissionRequest,
	PlanMode,
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
	setPermissionMode: vi.fn(),
	setPlanMode: vi.fn(),
	setModel: vi.fn(),
	respondPermission: vi.fn(),
}));

vi.mock("@/contexts/AgentChatContext", () => ({
	useAgentChatContext: () => mocks.useAgentChatContext(),
}));

vi.mock("./ChatSessionView", () => ({
	ChatSessionView: ({
		session,
		permissionMode,
		planMode,
		pendingPermission,
		onRespondPermission,
		onSend,
		stallObservation,
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
		stallObservation?: AgentStallObservation | null;
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
			<span data-testid={`stall-${session.id}`}>
				{stallObservation
					? `${stallObservation.turnPhase}:${stallObservation.idleSecs}`
					: "none"}
			</span>
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
) {
	mocks.useAgentChatContext.mockReturnValue({
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
		getSessionStallObservation: (sessionId: string) =>
			stallObservations[sessionId] ?? null,
		getSessionRuntimeSlashCommands: vi.fn().mockReturnValue([]),
		availableModels: [],
		availableModelsByBackend: {},
		error: null,
		sendMessage: mocks.sendMessage,
		interrupt: mocks.interrupt,
		cancelQueuedTurn: mocks.cancelQueuedTurn,
		setPermissionMode: mocks.setPermissionMode,
		setPlanMode: mocks.setPlanMode,
		setModel: mocks.setModel,
		respondPermission: mocks.respondPermission,
	});
}

describe("BoundSessionChat", () => {
	beforeEach(() => {
		for (const mock of Object.values(mocks)) {
			mock.mockClear();
		}
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
});
