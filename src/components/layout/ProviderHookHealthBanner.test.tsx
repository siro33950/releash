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
		states.publish("provider-hook-health", [
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
		]);

		render(<ProviderHookHealthBanner />);

		const warning = screen.getByRole("alert");
		expect(warning).toHaveTextContent("Claude, Codex");
		expect(screen.getAllByRole("alert")).toHaveLength(1);
	});

	it("後続SessionStartでbackend healthが解消されたら警告を消す", () => {
		render(<ProviderHookHealthBanner />);
		expect(screen.queryByRole("alert")).toBeNull();

		act(() =>
			states.publish("provider-hook-health", [
				{
					provider: "codex",
					launchId: "launch-codex",
					reason: "hook_unavailable",
				},
			]),
		);
		expect(screen.getByRole("alert")).toBeVisible();

		act(() => states.publish("provider-hook-health", []));
		expect(screen.queryByRole("alert")).toBeNull();
	});
});
