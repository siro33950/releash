import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	notifyAgentSessionChanged,
	subscribeAgentSessionChanged,
} from "./agentSessionEvents";

const clientEvents = vi.hoisted(() => ({
	handlers: new Map<
		string,
		(event: { payload: { worktreePath?: string } | null }) => void
	>(),
	unlisten: vi.fn(),
}));

vi.mock("@/lib/client", () => ({
	listenClient: (
		eventName: string,
		handler: (event: { payload: { worktreePath?: string } | null }) => void,
	) => {
		clientEvents.handlers.set(eventName, handler);
		return Promise.resolve(clientEvents.unlisten);
	},
}));

describe("agentSessionEvents", () => {
	beforeEach(() => {
		clientEvents.handlers.clear();
		clientEvents.unlisten.mockClear();
	});

	it("windowイベントのdetailをlistenerへ届ける", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeAgentSessionChanged(listener);

		notifyAgentSessionChanged("/repo/worktree");

		expect(listener).toHaveBeenCalledWith({ worktreePath: "/repo/worktree" });
		unsubscribe();
	});

	it("backendイベントのpayloadをlistenerへ届ける", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeAgentSessionChanged(listener);
		const handler = clientEvents.handlers.get("agent-session-changed");
		expect(handler).toBeDefined();

		handler?.({ payload: { worktreePath: "/repo/worktree" } });

		expect(listener).toHaveBeenCalledWith({ worktreePath: "/repo/worktree" });
		unsubscribe();
	});

	it("payload欠落時は空のdetailへfallbackする", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeAgentSessionChanged(listener);
		const handler = clientEvents.handlers.get("agent-session-changed");

		handler?.({ payload: null });

		expect(listener).toHaveBeenCalledWith({});
		unsubscribe();
	});

	it("解除でwindow購読とbackend購読を両方解く", async () => {
		const listener = vi.fn();
		const unsubscribe = subscribeAgentSessionChanged(listener);

		unsubscribe();

		notifyAgentSessionChanged("/repo/worktree");
		expect(listener).not.toHaveBeenCalled();
		await vi.waitFor(() =>
			expect(clientEvents.unlisten).toHaveBeenCalledOnce(),
		);
	});
});
