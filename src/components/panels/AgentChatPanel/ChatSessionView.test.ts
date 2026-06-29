import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/types/session";
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
	onLoadOlderMessages?: () => Promise<void>;
	onEvictOlderMessages?: (options: {
		oldestVisibleIndex?: number;
		onEvicted?: (eviction: { count: number; direction: "older" }) => void;
	}) => void;
}

function chatSessionViewElement({
	testSession = session,
	onLoadOlderMessages = vi.fn().mockResolvedValue(undefined),
	onEvictOlderMessages,
}: RenderOptions = {}) {
	return createElement(ChatSessionView, {
		session: testSession,
		isStreaming: false,
		isInterrupting: false,
		activityStatus: null,
		error: null,
		permissionMode: "edit",
		planMode: false,
		availableModels: [],
		selectedModel: "claude:sonnet",
		pendingQueue: [],
		selectedBackendId: null,
		canChangeBackend: false,
		worktreePath: "/repo",
		onSend: vi.fn().mockResolvedValue(undefined),
		onInterrupt: vi.fn(),
		onCancelQueuedTurn: vi.fn().mockResolvedValue(undefined),
		onLoadOlderMessages,
		onEvictOlderMessages,
		onPermissionModeChange: vi.fn(),
		onPlanModeChange: vi.fn(),
		onModelChange: vi.fn(),
		onRespondPermission: vi.fn(),
	});
}

function renderChatSessionView(options: RenderOptions = {}) {
	return render(chatSessionViewElement(options));
}

beforeEach(() => {
	mockInvoke.mockReset();
	mockInvoke.mockResolvedValue(null);
	mockVirtualRange.startIndex = 0;
	mockVirtualRange.endIndex = null;
	mockOffsetOverrides.clear();
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
