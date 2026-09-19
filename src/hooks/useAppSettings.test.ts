import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient, onClientRefresh } from "@/lib/client";
import { useBackgroundConfig } from "./useAppSettings";

const settings = {
	close_to_tray: true,
	auto_launch: false,
	start_minimized: true,
};

const serverSettings = {
	...settings,
	last_root_path: "",
	last_repo_paths: [],
	external_editor: "",
};

describe("window preferences", () => {
	beforeEach(() => {
		vi.clearAllMocks();
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
		vi.mocked(invokeClient).mockImplementation(async (command) =>
			command === "get_app_settings" ? serverSettings : undefined,
		);
	});

	it("loads and saves daemon settings without relaying shell preferences", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(invokeTauri).not.toHaveBeenCalledWith(
			"apply_desktop_settings",
			expect.anything(),
		);
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
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(invokeTauri).not.toHaveBeenCalledWith(
			"apply_desktop_settings",
			expect.anything(),
		);
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
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(invokeTauri).not.toHaveBeenCalledWith(
			"apply_desktop_settings",
			expect.anything(),
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
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(invokeTauri).not.toHaveBeenCalledWith(
			"apply_desktop_settings",
			expect.anything(),
		);
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
		expect(result.current.isDirty).toBe(true);
	});

	it.each(["get_login_item_status", "get_app_settings"])(
		"%s の初期取得失敗後の保存は失敗を通知し未保存の編集を保持する",
		async (command) => {
			if (command === "get_login_item_status") {
				vi.mocked(invokeTauri).mockRejectedValueOnce(new Error("load failed"));
			} else {
				vi.mocked(invokeClient).mockRejectedValueOnce(new Error("load failed"));
			}
			const { result } = renderHook(() => useBackgroundConfig());
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

	it("refreshes settings after reconnect while preserving an unsaved draft", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.setDraft({ ...settings, start_minimized: false }));
		vi.mocked(invokeClient).mockResolvedValueOnce({
			...serverSettings,
			close_to_tray: false,
		});
		act(() => {
			const calls = vi.mocked(onClientRefresh).mock.calls;
			calls[calls.length - 1][0]();
		});
		await waitFor(() => expect(invokeClient).toHaveBeenCalledTimes(2));
		expect(invokeTauri).toHaveBeenCalledWith("get_login_item_status");
		expect(invokeTauri).not.toHaveBeenCalledWith(
			"apply_desktop_settings",
			expect.anything(),
		);
		expect(
			vi
				.mocked(invokeTauri)
				.mock.calls.every(([command]) => command === "get_login_item_status"),
		).toBe(true);
		expect(result.current.draft).toEqual({
			...settings,
			start_minimized: false,
		});
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
	vi.mocked(invokeClient).mockResolvedValue(serverSettings);
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
	vi.mocked(invokeClient).mockResolvedValue(serverSettings);
	vi.mocked(invokeTauri).mockClear().mockResolvedValue({
		enabled: false,
		requested: false,
		requiresApproval: false,
		reason: null,
	});
	const { result } = renderHook(() => useBackgroundConfig());
	await waitFor(() => expect(result.current.loading).toBe(false));
	expect(invokeTauri).not.toHaveBeenCalledWith("install_cli");
	vi.mocked(invokeTauri).mockRejectedValueOnce(
		new Error("authorization denied"),
	);
	await act(() => result.current.installCli());
	expect(result.current.error).toBe("authorization denied");
	vi.mocked(invokeTauri).mockResolvedValueOnce(
		"Installed /usr/local/bin/releash",
	);
	await act(() => result.current.installCli());
	expect(result.current.cliMessage).toContain("/usr/local/bin/releash");
});
it("承認待ちの間に別項目を保存してもログイン登録の希望を失わない", async () => {
	vi.mocked(invokeClient)
		.mockReset()
		.mockImplementation(async (command) =>
			command === "get_app_settings"
				? { ...serverSettings, auto_launch: true }
				: undefined,
		);
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
