import { invoke } from "@tauri-apps/api/core";
import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderHookHealthBanner } from "./ProviderHookHealthBanner";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockInvoke = vi.mocked(invoke);

describe("ProviderHookHealthBanner", () => {
	beforeEach(() => {
		vi.useFakeTimers();
		mockInvoke.mockReset();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it("Provider別の未解消healthをアプリ全体の警告一つに集約する", async () => {
		mockInvoke.mockResolvedValueOnce([
			{
				provider: "claude",
				launchId: "launch-claude",
				reason: "provider_hook_configuration_rejected",
			},
			{
				provider: "codex",
				launchId: "launch-codex",
				reason: "codex_hook_delivery_unconfirmed",
			},
		]);

		render(<ProviderHookHealthBanner />);
		await act(async () => {
			await Promise.resolve();
		});

		const warning = screen.getByRole("alert");
		expect(warning).toHaveTextContent("Claude, Codex");
		expect(screen.getAllByRole("alert")).toHaveLength(1);
	});

	it("後続SessionStartでbackend healthが解消されたら警告を消す", async () => {
		mockInvoke
			.mockResolvedValueOnce([
				{
					provider: "codex",
					launchId: "launch-codex",
					reason: "codex_hook_delivery_unconfirmed",
				},
			])
			.mockResolvedValueOnce([]);

		render(<ProviderHookHealthBanner />);
		await act(async () => {
			await Promise.resolve();
		});
		expect(screen.getByRole("alert")).toBeVisible();

		await act(async () => {
			await vi.advanceTimersByTimeAsync(5_000);
		});
		expect(screen.queryByRole("alert")).toBeNull();
	});
});
