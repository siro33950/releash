import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	notifyProviderAgentSessionChanged,
	subscribeProviderAgentSessionChanged,
} from "./providerAgentSessionEvents";

const tauriEvents = vi.hoisted(() => ({
	handlers: new Map<
		string,
		(event: { payload: { worktreePath?: string } | null }) => void
	>(),
	unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (
		eventName: string,
		handler: (event: { payload: { worktreePath?: string } | null }) => void,
	) => {
		tauriEvents.handlers.set(eventName, handler);
		return Promise.resolve(tauriEvents.unlisten);
	},
}));

describe("providerAgentSessionEvents", () => {
	beforeEach(() => {
		tauriEvents.handlers.clear();
		tauriEvents.unlisten.mockClear();
	});

	it("windowイベントのdetailをlistenerへ届ける", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeProviderAgentSessionChanged(listener);

		notifyProviderAgentSessionChanged("/repo/worktree");

		expect(listener).toHaveBeenCalledWith({ worktreePath: "/repo/worktree" });
		unsubscribe();
	});

	it("backendイベントのpayloadをlistenerへ届ける", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeProviderAgentSessionChanged(listener);
		const handler = tauriEvents.handlers.get("provider-agent-session-changed");
		expect(handler).toBeDefined();

		handler?.({ payload: { worktreePath: "/repo/worktree" } });

		expect(listener).toHaveBeenCalledWith({ worktreePath: "/repo/worktree" });
		unsubscribe();
	});

	it("payload欠落時は空のdetailへfallbackする", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeProviderAgentSessionChanged(listener);
		const handler = tauriEvents.handlers.get("provider-agent-session-changed");

		handler?.({ payload: null });

		expect(listener).toHaveBeenCalledWith({});
		unsubscribe();
	});

	it("解除でwindow購読とbackend購読を両方解く", async () => {
		const listener = vi.fn();
		const unsubscribe = subscribeProviderAgentSessionChanged(listener);

		unsubscribe();

		notifyProviderAgentSessionChanged("/repo/worktree");
		expect(listener).not.toHaveBeenCalled();
		await vi.waitFor(() => expect(tauriEvents.unlisten).toHaveBeenCalledOnce());
	});
});
