import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient } from "@/lib/client";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useBackgroundConfig } from "./useAppSettings";

const states = stateSubscriptions();
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

const settings = {
	close_to_tray: true,
	auto_launch: false,
	start_minimized: true,
};

const desktopSettings = {
	closeToTray: true,
	startMinimized: true,
	crashReporting: false,
	performanceTelemetry: false,
	autoLaunch: false,
};

describe("window preferences", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		states.clear();
		states.publish("desktop-settings", desktopSettings);
		vi.mocked(invokeTauri).mockImplementation(async (command, args) => ({
			enabled:
				command === "set_login_item_enabled" &&
				Boolean(args && "enabled" in args && args.enabled),
			requested:
				command === "set_login_item_enabled" &&
				Boolean(args && "enabled" in args && args.enabled),
			requiresApproval: false,
			reason: null,
		}));
		vi.mocked(invokeClient).mockResolvedValue(undefined as never);
	});

	it("loads daemon settings from the subscription and saves without relaying shell preferences", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.draft).toEqual(settings);
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
		act(() => result.current.setDraft({ ...settings, close_to_tray: false }));
		await act(() => result.current.save());
		expect(invokeClient).toHaveBeenCalledWith("update_app_settings", {
			app: { close_to_tray: false, start_minimized: true },
		});
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
	});

	it("reports autostart failures without relaying window preferences", async () => {
		vi.mocked(invokeTauri).mockRejectedValueOnce(
			new Error("autostart unavailable"),
		);
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() =>
			expect(result.current.error).toBe("autostart unavailable"),
		);
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
	});

	it("keeps shell preferences when daemon rejects the save", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		vi.mocked(invokeClient).mockRejectedValueOnce(new Error("save failed"));
		act(() => result.current.setDraft({ ...settings, close_to_tray: false }));
		await act(async () => {
			await expect(result.current.save()).rejects.toThrow("save failed");
		});
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
		expect(result.current.isDirty).toBe(true);
	});

	it.each(["get_login_item_status", "desktop-settings"])(
		"%s の初期取得失敗後の保存は失敗を通知し未保存の編集を保持する",
		async (source) => {
			if (source === "get_login_item_status") {
				vi.mocked(invokeTauri).mockRejectedValueOnce(new Error("load failed"));
			} else {
				states.clear();
			}
			const { result } = renderHook(() => useBackgroundConfig());
			if (source === "desktop-settings")
				act(() => states.fail("desktop-settings", new Error("load failed")));
			await waitFor(() => expect(result.current.loading).toBe(false));
			expect(result.current.error).toBe("load failed");
			const draft = { ...settings, close_to_tray: false };
			act(() => result.current.setDraft(draft));
			vi.clearAllMocks();

			await act(async () => {
				await expect(result.current.save()).rejects.toThrow(
					"Background settings are not loaded.",
				);
			});

			expect(result.current.error).toBe("Background settings are not loaded.");
			expect(result.current.draft).toEqual(draft);
			expect(result.current.isDirty).toBe(true);
			expect(result.current.saving).toBe(false);
			expect(invokeClient).not.toHaveBeenCalled();
			expect(invokeTauri).not.toHaveBeenCalled();
		},
	);

	it("購読で届いた設定は未保存の編集を上書きしない", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.setDraft({ ...settings, start_minimized: false }));
		act(() =>
			states.publish("desktop-settings", {
				...desktopSettings,
				closeToTray: false,
			}),
		);
		expect(result.current.draft).toEqual({
			...settings,
			start_minimized: false,
		});
		expect(invokeClient).not.toHaveBeenCalled();
	});

	it("購読で届いた設定は編集が無ければ表示へ反映する", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() =>
			states.publish("desktop-settings", {
				...desktopSettings,
				closeToTray: false,
			}),
		);
		expect(result.current.draft.close_to_tray).toBe(false);
		expect(result.current.isDirty).toBe(false);
	});

	it("ログイン項目の有効化と無効化をRustへ渡し失敗時は保存しない", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.setDraft({ ...settings, auto_launch: true }));
		await act(() => result.current.save());
		expect(invokeTauri).toHaveBeenCalledWith("set_login_item_enabled", {
			enabled: true,
		});
		act(() => result.current.setDraft(settings));
		vi.mocked(invokeTauri).mockRejectedValueOnce(
			new Error("registration failed"),
		);
		const before = vi.mocked(invokeClient).mock.calls.length;
		await act(async () => {
			await expect(result.current.save()).rejects.toThrow(
				"registration failed",
			);
		});
		expect(invokeTauri).toHaveBeenCalledWith("set_login_item_enabled", {
			enabled: false,
		});
		expect(invokeClient).toHaveBeenCalledTimes(before);
		expect(result.current.isDirty).toBe(true);
	});
});

it("承認待ちは無効と表示し設定への導線を呼び出せる", async () => {
	states.clear();
	states.publish("desktop-settings", desktopSettings);
	vi.mocked(invokeTauri).mockResolvedValue({
		enabled: false,
		requested: true,
		requiresApproval: true,
		reason: null,
	});
	const { result } = renderHook(() => useBackgroundConfig());
	await waitFor(() => expect(result.current.loading).toBe(false));
	expect(result.current.draft.auto_launch).toBe(false);
	expect(result.current.loginItem?.requiresApproval).toBe(true);
	await act(() => result.current.openLoginSettings());
	expect(invokeTauri).toHaveBeenCalledWith("open_login_item_settings");
});
it("CLI設置は読み込み時に行わず明示操作だけで実行し失敗理由を表示する", async () => {
	states.clear();
	states.publish("desktop-settings", desktopSettings);
	vi.mocked(invokeTauri).mockClear().mockResolvedValue({
		enabled: false,
		requested: false,
		requiresApproval: false,
		reason: null,
	});
	const { result } = renderHook(() => useBackgroundConfig());
	await waitFor(() => expect(result.current.loading).toBe(false));
	expect(invokeClient).not.toHaveBeenCalledWith("install_cli");
	vi.mocked(invokeClient).mockRejectedValueOnce(
		new Error("authorization denied"),
	);
	await act(() => result.current.installCli());
	expect(result.current.error).toBe("authorization denied");
	vi.mocked(invokeClient).mockResolvedValueOnce({
		status: "installed",
		path: "/usr/local/bin/releash",
	} as never);
	await act(() => result.current.installCli());
	expect(result.current.cliMessage).toContain("/usr/local/bin/releash");
});
it("承認待ちの間に別項目を保存してもログイン登録の希望を失わない", async () => {
	states.clear();
	states.publish("desktop-settings", { ...desktopSettings, autoLaunch: true });
	vi.mocked(invokeClient)
		.mockReset()
		.mockResolvedValue(undefined as never);
	vi.mocked(invokeTauri).mockReset().mockResolvedValue({
		enabled: false,
		requested: true,
		requiresApproval: true,
		reason: null,
	});
	const { result } = renderHook(() => useBackgroundConfig());
	await waitFor(() => expect(result.current.loading).toBe(false));
	act(() =>
		result.current.setDraft({
			...result.current.draft,
			start_minimized: false,
		}),
	);
	await act(() => result.current.save());
	expect(invokeClient).toHaveBeenCalledWith("update_app_settings", {
		app: { close_to_tray: true, start_minimized: false },
	});
	expect(result.current.draft.auto_launch).toBe(false);
	expect(invokeTauri).not.toHaveBeenCalledWith(
		"set_login_item_enabled",
		expect.anything(),
	);
});

it.each(["installed", "alreadyInstalled"] as const)(
	"CLI の設置結果 %s をサーバから受け取り表示する",
	async (status) => {
		states.publish("desktop-settings", desktopSettings);
		vi.mocked(invokeTauri).mockResolvedValue({
			enabled: false,
			requiresApproval: false,
			reason: null,
		});
		vi.mocked(invokeClient).mockResolvedValue({
			status,
			path: "/usr/local/bin/releash",
		} as never);
		const { result } = renderHook(() => useBackgroundConfig());
		await act(() => result.current.installCli());
		expect(invokeClient).toHaveBeenCalledWith("install_cli");
		expect(result.current.cliMessage).toBe(
			`Releash CLI ${status === "installed" ? "installed" : "already installed"} at /usr/local/bin/releash`,
		);
	},
);

it("CLI の配置拒否をエラーとして表示し以前の成功表示を消す", async () => {
	states.publish("desktop-settings", desktopSettings);
	vi.mocked(invokeTauri).mockResolvedValue({
		enabled: false,
		requiresApproval: false,
		reason: null,
	});
	vi.mocked(invokeClient).mockResolvedValueOnce({
		status: "installed",
		path: "/usr/local/bin/releash",
	} as never);
	const { result } = renderHook(() => useBackgroundConfig());
	await act(() => result.current.installCli());
	vi.mocked(invokeClient).mockRejectedValueOnce(
		new Error("Move Releash.app to Applications before installing the CLI."),
	);
	await act(() => result.current.installCli());
	expect(result.current.cliMessage).toBeNull();
	expect(result.current.error).toBe(
		"Move Releash.app to Applications before installing the CLI.",
	);
});
