import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatSession, PermissionMode, PlanMode } from "@/types/session";

const mocks = vi.hoisted(() => ({
	useAgentChatContext: vi.fn(),
	loadSession: vi.fn().mockResolvedValue(null),
	loadOlderMessages: vi.fn().mockResolvedValue(undefined),
	evictOlderMessages: vi.fn().mockResolvedValue(undefined),
	registerViewableSession: vi.fn(() => vi.fn()),
	sendMessage: vi.fn().mockResolvedValue(undefined),
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
	}: {
		session: ChatSession;
		permissionMode: PermissionMode;
		planMode: PlanMode;
	}) => (
		<div data-testid={`chat-${session.id}`}>
			<span data-testid={`permission-${session.id}`}>{permissionMode}</span>
			<span data-testid={`plan-${session.id}`}>{String(planMode)}</span>
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

function setContext(sessionsById: Record<string, ChatSession>) {
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
		getSessionPendingQueue: vi.fn().mockReturnValue([]),
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
});
