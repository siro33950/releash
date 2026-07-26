import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionFeedbackEntry } from "@/hooks/useSessionStore";
import type {
	AgentStallObservation,
	ChatSession,
	PermissionRequest,
	QueuedAgentTurn,
	SessionNotice,
} from "@/types/session";
import { ChatSessionView } from "./ChatSessionView";

const { mockInvoke, mockVirtualRange, mockOffsetOverrides } = vi.hoisted(
	() => ({
		mockInvoke: vi.fn(),
		mockVirtualRange: { startIndex: 0, endIndex: null as number | null },
		mockOffsetOverrides: new Map<number, number>(),
	}),
);

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tanstack/react-virtual", () => ({
	useVirtualizer: ({
		count,
		estimateSize,
		getItemKey,
	}: {
		count: number;
		estimateSize: (index: number) => number;
		getItemKey?: (index: number) => string | number;
	}) => ({
		getVirtualItems: () =>
			Array.from(
				{
					length:
						Math.min(mockVirtualRange.endIndex ?? count - 1, count - 1) -
						Math.min(mockVirtualRange.startIndex, count) +
						1,
				},
				(_, offset) => {
					const index = mockVirtualRange.startIndex + offset;
					const size = estimateSize(index);
					return {
						index,
						key: getItemKey?.(index) ?? index,
						start: index * size,
						size,
						end: (index + 1) * size,
					};
				},
			).filter((item) => item.index >= 0 && item.index < count),
		getTotalSize: () => {
			let total = 0;
			for (let index = 0; index < count; index++) {
				total += estimateSize(index);
			}
			return total;
		},
		getOffsetForIndex: (targetIndex: number) => {
			const override = mockOffsetOverrides.get(targetIndex);
			if (override !== undefined) return [override, "start"] as const;
			let offset = 0;
			for (let index = 0; index < targetIndex; index++) {
				offset += estimateSize(index);
			}
			return [offset, "start"] as const;
		},
		measureElement: () => {},
		scrollToIndex: () => {},
	}),
}));

const clipboardWriteText = vi.fn().mockResolvedValue(undefined);
Object.defineProperty(navigator, "clipboard", {
	configurable: true,
	value: { writeText: clipboardWriteText },
});

const PERMISSION_RECONCILE_SESSION_ID = "permission-reconcile-session";

vi.hoisted(() => {
	globalThis.localStorage?.setItem(
		"releash.accepted-permission-response-operations.v1",
		JSON.stringify([
			[
				JSON.stringify([
					"permission-reconcile-session",
					"permission-request-1",
				]),
				"permission-operation-1",
			],
		]),
	);
});

const session: ChatSession = {
	id: "s1",
	worktreePath: "/repo",
	messages: [
		{
			id: "m1",
			role: "human",
			parts: [{ type: "text", content: "hello" }],
			timestamp: 1000,
		},
	],
	state: "active",
	createdAt: 1000,
	updatedAt: 1001,
	permissionMode: "edit",
};

interface RenderOptions {
	testSession?: ChatSession;
	isStreaming?: boolean;
	stallObservation?: AgentStallObservation | null;
	onLoadOlderMessages?: () => Promise<void>;
	onEvictOlderMessages?: (options: {
		oldestVisibleIndex?: number;
		onEvicted?: (eviction: { count: number; direction: "older" }) => void;
	}) => void;
	pendingPermission?: PermissionRequest | null;
	pendingQueue?: QueuedAgentTurn[];
	notice?: SessionNotice | null;
	feedback?: SessionFeedbackEntry[];
	onDismissFeedback?: (entry: SessionFeedbackEntry) => void;
	onRetryFeedback?: (entry: SessionFeedbackEntry) => void;
	error?: string | null;
	onDismissError?: () => void;
	queuePaused?: boolean;
	onResumeQueue?: () => Promise<void>;
	onRespondPermission?: (
		requestId: string,
		allow: boolean,
		updatedInput?: Record<string, unknown>,
	) => void;
}

function chatSessionViewElement({
	testSession = session,
	isStreaming = false,
	stallObservation = null,
	onLoadOlderMessages = vi.fn().mockResolvedValue(undefined),
	onEvictOlderMessages,
	pendingPermission = null,
	pendingQueue = [],
	notice = null,
	feedback = [],
	onDismissFeedback,
	onRetryFeedback,
	error = null,
	onDismissError = vi.fn(),
	queuePaused = false,
	onResumeQueue = vi.fn().mockResolvedValue(undefined),
	onRespondPermission = vi.fn(),
}: RenderOptions = {}) {
	return createElement(ChatSessionView, {
		session: testSession,
		isStreaming,
		isInterrupting: false,
		activityStatus: null,
		error,
		onDismissError,
		permissionMode: "edit",
		planMode: false,
		availableModels: [],
		backends: [],
		selectedModel: "claude:sonnet",
		pendingPermission,
		pendingQueue,
		queuePaused,
		stallObservation,
		notice,
		feedback,
		onDismissFeedback,
		onRetryFeedback,
		selectedBackendId: null,
		canChangeBackend: false,
		worktreePath: "/repo",
		onSend: vi.fn().mockResolvedValue(true),
		onInterrupt: vi.fn(),
		onCancelQueuedTurn: vi.fn().mockResolvedValue(undefined),
		onResumeQueue,
		onLoadOlderMessages,
		onEvictOlderMessages,
		onPermissionModeChange: vi.fn(),
		onPlanModeChange: vi.fn(),
		onModelChange: vi.fn(),
		onRespondPermission,
	});
}

function renderChatSessionView(options: RenderOptions = {}) {
	return render(chatSessionViewElement(options));
}

function askUserQuestionPresentation() {
	return {
		kind: "ask_user_question",
		canEditInput: false,
		canEditContent: false,
		canEditMultiEditContent: false,
		directContentEditLabel: null,
		directContent: "",
		multiEditReplacementContents: [],
		multiEditOldStrings: [],
		hasResolvedDetail: true,
		plan: "",
		allowedPrompts: [],
		questions: [
			{
				question: "Which library should we use?",
				header: "Library",
				options: [
					{ label: "React", description: "Popular UI framework" },
					{ label: "Vue", description: "Progressive framework" },
				],
				multiSelect: false,
			},
		],
	};
}

beforeEach(() => {
	mockInvoke.mockReset();
	mockInvoke.mockImplementation((command: string, args: unknown) => {
		if (command === "present_agent_permission_request") {
			const { requestId } = args as { requestId?: string };
			return Promise.resolve(
				requestId === "perm-question-1" ? askUserQuestionPresentation() : null,
			);
		}
		return Promise.resolve(null);
	});
	mockVirtualRange.startIndex = 0;
	mockVirtualRange.endIndex = null;
	mockOffsetOverrides.clear();
	clipboardWriteText.mockClear();
	Object.defineProperty(navigator, "clipboard", {
		configurable: true,
		value: { writeText: clipboardWriteText },
	});
});

const pendingPermission: PermissionRequest = {
	id: "perm-1",
	toolName: "Bash",
	kind: "tool_approval",
	input: { command: "echo hi" },
	title: "Run command",
};

describe("ChatSessionView error parts", () => {
	it("renders the live or reloaded error part as an Error block", () => {
		renderChatSessionView({
			testSession: {
				...session,
				state: "error",
				errorReason: "app server stopped",
				messages: [
					...session.messages,
					{
						id: "session-error",
						role: "agent",
						parts: [{ type: "error", content: "app server stopped" }],
						timestamp: 1002,
					},
				],
			},
		});

		expect(screen.getByText("app server stopped")).toBeInTheDocument();
	});
});

describe("ChatSessionView operation supervision", () => {
	it("surfaces an accepted permission response that later requires reconciliation", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
				case "list_pending_agent_recovery":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({ type: "current", plan: null });
				case "get_agent_permission_response_operation":
					return Promise.resolve({
						receipt: {
							operation_id: "permission-operation-1",
							session_id: PERMISSION_RECONCILE_SESSION_ID,
							request_id: "permission-request-1",
							input_ref: "permission-response:permission-request-1",
						},
						latest_status: {
							type: "reconciliation_required",
							failure: { kind: "storage_unavailable" },
						},
					});
				default:
					return Promise.resolve(null);
			}
		});

		renderChatSessionView({
			testSession: { ...session, id: PERMISSION_RECONCILE_SESSION_ID },
		});

		expect(
			await screen.findByText(/Accepted permission response requires/),
		).toHaveTextContent("permission-operation-1");
	});
});

describe("ChatSessionView session-local controls", () => {
	it("renders a dismissible operation error banner", () => {
		const onDismissError = vi.fn();
		renderChatSessionView({
			error: "send failed",
			onDismissError,
		});

		expect(screen.getByRole("alert")).toHaveTextContent("send failed");
		fireEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
		expect(onDismissError).toHaveBeenCalledOnce();
	});

	const sessionWithAgentResponse: ChatSession = {
		...session,
		messages: [
			...session.messages,
			{
				id: "m2",
				role: "agent",
				parts: [{ type: "text", content: "Latest agent response" }],
				timestamp: 1002,
			},
		],
	};
	it("renders text and image-only pending queue entries", () => {
		renderChatSessionView({
			pendingQueue: [
				{
					id: "queued-text",
					contentPreview: "Review the failing logs",
					createdAt: 1002,
					permissionMode: "edit",
					imageCount: 0,
				},
				{
					id: "queued-image",
					contentPreview: "",
					createdAt: 1003,
					permissionMode: "edit",
					imageCount: 1,
				},
			],
		});

		expect(screen.getByText("Queued 1")).toBeInTheDocument();
		expect(screen.getByText("Review the failing logs")).toBeInTheDocument();
		expect(screen.getByText("Queued 2")).toBeInTheDocument();
		expect(screen.getByText("[image]")).toBeInTheDocument();
	});

	it("renders the backend-owned persist notice as a session alert", () => {
		renderChatSessionView({
			notice: {
				sessionId: session.id,
				kind: "persist_failure",
				message: "Failed to save the completed response.",
				createdAt: 1_001,
			},
		});

		expect(screen.getByTestId("session-notice-banner")).toHaveTextContent(
			"Failed to save the completed response.",
		);
		expect(screen.getByRole("alert")).toBeInTheDocument();
	});

	it("renders an event log recovery notice as status without alert exposure", () => {
		renderChatSessionView({
			notice: {
				sessionId: session.id,
				kind: "event_log_recovered",
				message: "Recovered the damaged event log.",
				createdAt: 1_002,
			},
		});

		const banner = screen.getByTestId("session-notice-banner");
		expect(banner).toHaveTextContent("Recovered the damaged event log.");
		expect(screen.getByRole("status")).toBe(banner);
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});

	it("renders canonical feedback fields and dismisses the exact identity", () => {
		const onDismissFeedback = vi.fn();
		const feedback: SessionFeedbackEntry = {
			feedback_id: "feedback-1",
			attempt_id: "attempt-1",
			session_id: session.id,
			operation: "send",
			revision: "2",
			actions: ["dismiss"],
			action_identities: [
				{
					action: "dismiss",
					action_id: "dismiss-feedback-1",
					origin_revision: "2",
				},
			],
			failure: {
				kind: "persist_failure",
				retryable: true,
				label: "Send could not be saved",
				detail: "Retry after storage recovers.",
				correlation_id: "correlation-1",
			},
		};
		renderChatSessionView({ feedback: [feedback], onDismissFeedback });

		expect(screen.getByTestId("session-feedback-banner")).toHaveTextContent(
			"Send could not be saved",
		);
		fireEvent.click(screen.getByRole("button", { name: "Dismiss feedback" }));
		expect(onDismissFeedback).toHaveBeenCalledWith(feedback);
	});

	it("offers retry only when the backend feedback projection allows it", () => {
		const onRetryFeedback = vi.fn();
		const feedback: SessionFeedbackEntry = {
			feedback_id: "feedback-retry",
			attempt_id: "attempt-retry",
			session_id: session.id,
			operation: "send",
			revision: "4",
			actions: ["dismiss", "retry_resolution"],
			action_identities: [
				{
					action: "dismiss",
					action_id: "dismiss-feedback-retry",
					origin_revision: "4",
				},
				{
					action: "retry_resolution",
					action_id: "retry-feedback-retry",
					origin_revision: "4",
				},
			],
			failure: {
				kind: "storage_unavailable",
				retryable: true,
				label: "Storage unavailable",
				detail: null,
				correlation_id: "correlation-retry",
			},
		};
		renderChatSessionView({ feedback: [feedback], onRetryFeedback });

		fireEvent.click(screen.getByRole("button", { name: "Retry" }));
		expect(onRetryFeedback).toHaveBeenCalledWith(feedback);
	});

	it("shows the durable SessionClosed interruption on the reopened agent turn", () => {
		renderChatSessionView({
			testSession: {
				...sessionWithAgentResponse,
				lastTurnInterruption: {
					messageId: "m2",
					reason: "session_closed",
				},
			},
		});

		expect(screen.getByTestId("turn-interruption-chip")).toHaveTextContent(
			"Interrupted: Session closed",
		);
	});

	it("keeps find and raw scrollback available from the toolbar", async () => {
		const user = userEvent.setup();
		renderChatSessionView({ testSession: sessionWithAgentResponse });

		await user.click(
			screen.getByRole("button", { name: "Enable raw scrollback" }),
		);
		expect(
			screen.getByRole("button", { name: "Disable raw scrollback" }),
		).toBeInTheDocument();

		await user.click(
			screen.getByRole("button", { name: "Find in current thread" }),
		);
		expect(
			screen.getByPlaceholderText("Find in current thread"),
		).toBeInTheDocument();
	});

	it("keeps the fixed Cmd/Ctrl+F find shortcut", () => {
		renderChatSessionView({ testSession: sessionWithAgentResponse });

		fireEvent.keyDown(window, { key: "f", metaKey: true });

		expect(
			screen.getByPlaceholderText("Find in current thread"),
		).toBeInTheDocument();
	});

	it("keeps copy available from the toolbar", async () => {
		renderChatSessionView({ testSession: sessionWithAgentResponse });

		fireEvent.click(
			screen.getByRole("button", { name: "Copy latest agent response" }),
		);

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith("Latest agent response"),
		);
	});

	it("keeps the fixed Ctrl+O copy shortcut", async () => {
		renderChatSessionView({ testSession: sessionWithAgentResponse });

		fireEvent.keyDown(window, { key: "o", ctrlKey: true });

		await waitFor(() =>
			expect(clipboardWriteText).toHaveBeenCalledWith("Latest agent response"),
		);
	});
});

describe("ChatSessionView stall status", () => {
	it("shows the active stall observation while streaming", () => {
		renderChatSessionView({
			isStreaming: true,
			stallObservation: {
				turnPhase: "streaming",
				idleSecs: 65,
				signalCount: 1,
				capReached: false,
			},
		});

		expect(
			screen.getByText("No agent output for 1m 5s. Session remains active."),
		).toBeInTheDocument();
	});

	it("hides the stall observation while idle", () => {
		renderChatSessionView({
			isStreaming: false,
			stallObservation: {
				turnPhase: "streaming",
				idleSecs: 65,
				signalCount: 1,
				capReached: false,
			},
		});

		expect(screen.queryByText(/No agent output for/)).not.toBeInTheDocument();
	});

	it("hides the stall status without an observation", () => {
		renderChatSessionView({ isStreaming: true, stallObservation: null });

		expect(screen.queryByText(/No agent output for/)).not.toBeInTheDocument();
	});
});

describe("ChatSessionView scroll loading", () => {
	it("calls onLoadOlderMessages when scrollTop is below the threshold", () => {
		const onLoadOlderMessages = vi.fn().mockResolvedValue(undefined);
		renderChatSessionView({ onLoadOlderMessages });
		const scroll = screen.getByTestId("chat-session-scroll");

		Object.defineProperty(scroll, "scrollTop", {
			configurable: true,
			value: 79,
		});
		fireEvent.scroll(scroll);

		expect(onLoadOlderMessages).toHaveBeenCalledTimes(1);
	});

	it("does not call onLoadOlderMessages when scrollTop is at the threshold", () => {
		const onLoadOlderMessages = vi.fn().mockResolvedValue(undefined);
		renderChatSessionView({ onLoadOlderMessages });
		const scroll = screen.getByTestId("chat-session-scroll");

		Object.defineProperty(scroll, "scrollTop", {
			configurable: true,
			value: 80,
		});
		fireEvent.scroll(scroll);

		expect(onLoadOlderMessages).not.toHaveBeenCalled();
	});

	it("does not evict older messages while the oldest virtual row is still in overscan", () => {
		const onEvictOlderMessages = vi.fn();
		renderChatSessionView({ onEvictOlderMessages });
		const scroll = screen.getByTestId("chat-session-scroll");

		Object.defineProperty(scroll, "scrollTop", {
			configurable: true,
			value: 400,
		});
		fireEvent.scroll(scroll);

		expect(onEvictOlderMessages).not.toHaveBeenCalled();
	});

	it("passes the virtual range and preserves the scroll anchor with virtualizer offsets", () => {
		const messages = [
			{ ...session.messages[0], id: "m1", role: "human" as const },
			{
				...session.messages[0],
				id: "m2",
				role: "agent" as const,
				parts: [{ type: "text" as const, content: "agent" }],
			},
			{ ...session.messages[0], id: "m3", role: "human" as const },
			{ ...session.messages[0], id: "m4", role: "human" as const },
		];
		const expandedSession = { ...session, messages };
		const onEvictOlderMessages = vi.fn((options) => {
			options.onEvicted?.({ count: 2, direction: "older" });
		});
		mockVirtualRange.startIndex = 2;
		mockOffsetOverrides.set(2, 260);
		const view = renderChatSessionView({
			testSession: expandedSession,
			onEvictOlderMessages,
		});
		const scroll = screen.getByTestId("chat-session-scroll");
		Object.defineProperties(scroll, {
			scrollTop: { configurable: true, writable: true, value: 400 },
			scrollHeight: { configurable: true, value: 1000 },
			clientHeight: { configurable: true, value: 200 },
		});

		fireEvent.scroll(scroll);

		expect(onEvictOlderMessages).toHaveBeenCalledTimes(1);
		expect(onEvictOlderMessages.mock.calls[0]?.[0].oldestVisibleIndex).toBe(2);

		view.rerender(
			chatSessionViewElement({
				testSession: { ...expandedSession, messages: messages.slice(2) },
				onEvictOlderMessages,
			}),
		);

		expect(scroll.scrollTop).toBe(140);
	});

	it("requests eviction after loading older messages at the top", async () => {
		const onLoadOlderMessages = vi.fn().mockResolvedValue(undefined);
		const onEvictOlderMessages = vi.fn();
		const initialMessages = Array.from({ length: 4 }, (_, index) => ({
			...session.messages[0],
			id: `m${index + 5}`,
		}));
		const olderMessages = Array.from({ length: 4 }, (_, index) => ({
			...session.messages[0],
			id: `m${index + 1}`,
		}));
		mockVirtualRange.startIndex = 0;
		mockVirtualRange.endIndex = 3;
		const view = renderChatSessionView({
			testSession: { ...session, messages: initialMessages },
			onLoadOlderMessages,
			onEvictOlderMessages,
		});
		const scroll = screen.getByTestId("chat-session-scroll");
		Object.defineProperty(scroll, "scrollTop", {
			configurable: true,
			value: 20,
		});

		fireEvent.scroll(scroll);
		expect(onLoadOlderMessages).toHaveBeenCalledTimes(1);

		view.rerender(
			chatSessionViewElement({
				testSession: {
					...session,
					messages: [...olderMessages, ...initialMessages],
				},
				onLoadOlderMessages,
				onEvictOlderMessages,
			}),
		);

		await waitFor(() => {
			expect(onEvictOlderMessages).toHaveBeenCalledWith(
				expect.objectContaining({
					oldestVisibleIndex: 0,
				}),
			);
		});
	});

	it("requests eviction when a new tail message arrives while following the bottom", async () => {
		const onEvictOlderMessages = vi.fn();
		const messages = Array.from({ length: 4 }, (_, index) => ({
			...session.messages[0],
			id: `m${index + 1}`,
		}));
		mockVirtualRange.startIndex = 2;
		const view = renderChatSessionView({
			testSession: { ...session, messages },
			onEvictOlderMessages,
		});

		view.rerender(
			chatSessionViewElement({
				testSession: {
					...session,
					messages: [
						...messages,
						{ ...session.messages[0], id: "m5", timestamp: 1005 },
					],
				},
				onEvictOlderMessages,
			}),
		);

		await waitFor(() => {
			expect(onEvictOlderMessages).toHaveBeenCalledWith(
				expect.objectContaining({
					oldestVisibleIndex: 2,
				}),
			);
		});
	});

	it("does not request eviction for a new tail message while the user is away from the bottom", async () => {
		const onEvictOlderMessages = vi.fn();
		const messages = Array.from({ length: 4 }, (_, index) => ({
			...session.messages[0],
			id: `m${index + 1}`,
		}));
		mockVirtualRange.startIndex = 0;
		const view = renderChatSessionView({
			testSession: { ...session, messages },
			onEvictOlderMessages,
		});
		const scroll = screen.getByTestId("chat-session-scroll");
		Object.defineProperties(scroll, {
			scrollTop: { configurable: true, value: 400 },
			scrollHeight: { configurable: true, value: 1000 },
			clientHeight: { configurable: true, value: 200 },
		});

		fireEvent.scroll(scroll);
		onEvictOlderMessages.mockClear();
		mockVirtualRange.startIndex = 2;
		view.rerender(
			chatSessionViewElement({
				testSession: {
					...session,
					messages: [
						...messages,
						{ ...session.messages[0], id: "m5", timestamp: 1005 },
					],
				},
				onEvictOlderMessages,
			}),
		);
		await new Promise((resolve) => setTimeout(resolve, 0));

		expect(onEvictOlderMessages).not.toHaveBeenCalled();
	});
});

describe("ChatSessionView pending permission fallback", () => {
	it("renders pending permission when no message permission part exists", async () => {
		renderChatSessionView({ pendingPermission });

		const dialogs = await screen.findAllByTestId("permission-dialog");

		expect(dialogs).toHaveLength(1);
		expect(screen.getByText("Permission required: Run command")).toBeTruthy();
	});

	it("tail-follows when the fallback pending permission appears", async () => {
		const view = renderChatSessionView();
		const scroll = screen.getByTestId("chat-session-scroll");
		Object.defineProperties(scroll, {
			scrollTop: { configurable: true, writable: true, value: 800 },
			scrollHeight: { configurable: true, writable: true, value: 1000 },
			clientHeight: { configurable: true, value: 200 },
		});
		fireEvent.scroll(scroll);
		Object.defineProperty(scroll, "scrollHeight", {
			configurable: true,
			writable: true,
			value: 1200,
		});

		view.rerender(chatSessionViewElement({ pendingPermission }));

		await waitFor(() => {
			expect(scroll.scrollTop).toBe(1000);
		});
	});

	it("does not render a duplicate when the message already has the permission part", async () => {
		const testSession: ChatSession = {
			...session,
			messages: [
				...session.messages,
				{
					id: "m2",
					role: "agent",
					parts: [
						{
							type: "permission",
							request: pendingPermission,
							status: "pending",
						},
					],
					timestamp: 1002,
				},
			],
		};

		renderChatSessionView({ testSession, pendingPermission });

		const dialogs = await screen.findAllByTestId("permission-dialog");

		expect(dialogs).toHaveLength(1);
	});

	it("routes fallback Allow through onRespondPermission", async () => {
		const onRespondPermission = vi.fn();
		renderChatSessionView({ pendingPermission, onRespondPermission });

		await userEvent.click(await screen.findByText("Allow"));

		expect(onRespondPermission.mock.calls[0]).toEqual([
			"perm-1",
			true,
			undefined,
		]);
	});

	it("routes fallback Deny through onRespondPermission", async () => {
		const onRespondPermission = vi.fn();
		renderChatSessionView({ pendingPermission, onRespondPermission });

		await userEvent.click(await screen.findByText("Deny"));

		expect(onRespondPermission.mock.calls[0]).toEqual(["perm-1", false]);
	});

	it("routes fallback AskUserQuestion answers through onRespondPermission with merged input", async () => {
		const onRespondPermission = vi.fn();
		const questionPermission: PermissionRequest = {
			id: "perm-question-1",
			toolName: "AskUserQuestion",
			kind: "question",
			input: { source: "fallback" },
			title: "Question",
		};
		renderChatSessionView({
			pendingPermission: questionPermission,
			onRespondPermission,
		});

		await userEvent.click(await screen.findByText("React"));
		await userEvent.click(screen.getByText("Submit"));

		expect(onRespondPermission.mock.calls[0]).toEqual([
			"perm-question-1",
			true,
			{
				source: "fallback",
				answers: {
					"Which library should we use?": "React",
				},
			},
		]);
	});

	it("offers an explicit resume action for a paused queue", async () => {
		const onResumeQueue = vi.fn().mockResolvedValue(undefined);
		renderChatSessionView({
			pendingQueue: [
				{
					id: "queued-1",
					contentPreview: "follow up",
					createdAt: 1002,
					permissionMode: "edit",
					imageCount: 0,
				},
			],
			queuePaused: true,
			onResumeQueue,
		});

		await userEvent.click(screen.getByRole("button", { name: "Resume queue" }));

		expect(onResumeQueue).toHaveBeenCalledOnce();
	});
});
