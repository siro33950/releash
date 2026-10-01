import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { ProviderHookHealthBanner } from "./ProviderHookHealthBanner";

const states = stateSubscriptions();
vi.mock("@/lib/client", () => ({
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

describe("ProviderHookHealthBanner", () => {
	beforeEach(() => {
		states.clear();
	});

	it("Provider別の未解消healthをアプリ全体の警告一つに集約する", () => {
		states.publish("provider-hook-health", {
			readErrors: [],
			warnings: [
				{
					provider: "claude",
					launchId: "launch-claude",
					reason: "hook_unavailable",
				},
				{
					provider: "codex",
					launchId: "launch-codex",
					reason: "hook_unavailable",
				},
			],
		});

		render(<ProviderHookHealthBanner />);

		const warning = screen.getByRole("alert");
		expect(warning).toHaveTextContent("Claude, Codex");
		expect(screen.getAllByRole("alert")).toHaveLength(1);
	});

	it("後続SessionStartでbackend healthが解消されたら警告を消す", () => {
		render(<ProviderHookHealthBanner />);
		expect(screen.queryByRole("alert")).toBeNull();

		act(() =>
			states.publish("provider-hook-health", {
				readErrors: [],
				warnings: [
					{
						provider: "codex",
						launchId: "launch-codex",
						reason: "hook_unavailable",
					},
				],
			}),
		);
		expect(screen.getByRole("alert")).toBeVisible();

		act(() =>
			states.publish("provider-hook-health", { readErrors: [], warnings: [] }),
		);
		expect(screen.queryByRole("alert")).toBeNull();
	});
	it("記録の読取失敗を警告なしと区別し回復後に解除する", () => {
		render(<ProviderHookHealthBanner />);
		act(() => states.fail("provider-hook-health", new Error("corrupt record")));
		expect(screen.getByRole("alert")).toHaveTextContent("corrupt record");
		act(() =>
			states.publish("provider-hook-health", { readErrors: [], warnings: [] }),
		);
		expect(screen.queryByRole("alert")).not.toBeInTheDocument();
	});
});

it("読めた警告と記録ごとの失敗を一緒に出し購読失敗後も警告を残す", () => {
	states.clear();
	states.publish("provider-hook-health", {
		warnings: [
			{
				provider: "codex",
				launchId: "launch",
				reason: "local_api_unavailable",
			},
		],
		readErrors: ["record unreadable"],
	});
	render(<ProviderHookHealthBanner />);
	expect(screen.getByRole("alert")).toHaveTextContent("Codex");
	expect(screen.getByRole("alert")).toHaveTextContent("record unreadable");
	act(() =>
		states.fail("provider-hook-health", new Error("subscription unavailable")),
	);
	expect(screen.getByRole("alert")).toHaveTextContent("Codex");
	expect(screen.getByRole("alert")).toHaveTextContent(
		"subscription unavailable",
	);
});
