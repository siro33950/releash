import { fireEvent, render, screen } from "@testing-library/react";
import { createElement } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatSession } from "@/types/session";
import {
	ChatSessionView,
	shouldTailFollowMessageChange,
} from "./ChatSessionView";

const { mockInvoke } = vi.hoisted(() => ({ mockInvoke: vi.fn() }));

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
			Array.from({ length: count }, (_, index) => {
				const size = estimateSize(index);
				return {
					index,
					key: getItemKey?.(index) ?? index,
					start: index * size,
					size,
					end: (index + 1) * size,
				};
			}),
		getTotalSize: () => {
			let total = 0;
			for (let index = 0; index < count; index++) {
				total += estimateSize(index);
			}
			return total;
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

function renderChatSessionView(
	onLoadOlderMessages: () => Promise<void> = vi
		.fn()
		.mockResolvedValue(undefined),
) {
	return render(
		createElement(ChatSessionView, {
			session,
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
			onPermissionModeChange: vi.fn(),
			onPlanModeChange: vi.fn(),
			onModelChange: vi.fn(),
			onRespondPermission: vi.fn(),
		}),
	);
}

beforeEach(() => {
	mockInvoke.mockReset();
	mockInvoke.mockResolvedValue(null);
});

describe("shouldTailFollowMessageChange", () => {
	it("does not tail-follow when older messages are prepended", () => {
		expect(
			shouldTailFollowMessageChange(
				["m3", "m4"],
				["m1", "m2", "m3", "m4"],
				true,
			),
		).toBe(false);
	});

	it("tail-follows when new messages are appended", () => {
		expect(
			shouldTailFollowMessageChange(["m1", "m2"], ["m1", "m2", "m3"], false),
		).toBe(true);
	});

	it("tail-follows initial hydration", () => {
		expect(shouldTailFollowMessageChange([], ["m1"], false)).toBe(true);
	});
});

describe("ChatSessionView scroll loading", () => {
	it("calls onLoadOlderMessages when scrollTop is below the threshold", () => {
		const onLoadOlderMessages = vi.fn().mockResolvedValue(undefined);
		renderChatSessionView(onLoadOlderMessages);
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
		renderChatSessionView(onLoadOlderMessages);
		const scroll = screen.getByTestId("chat-session-scroll");

		Object.defineProperty(scroll, "scrollTop", {
			configurable: true,
			value: 80,
		});
		fireEvent.scroll(scroll);

		expect(onLoadOlderMessages).not.toHaveBeenCalled();
	});
});
