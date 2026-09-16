import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { isEnabled } from "@tauri-apps/plugin-autostart";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient, onClientRefresh } from "@/lib/clientSocket";
import { useBackgroundConfig } from "./useAppSettings";

vi.mock("@tauri-apps/plugin-autostart", () => ({
	isEnabled: vi.fn().mockResolvedValue(false),
	enable: vi.fn().mockResolvedValue(undefined),
	disable: vi.fn().mockResolvedValue(undefined),
}));

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
		vi.mocked(invokeTauri).mockResolvedValue(undefined);
		vi.mocked(invokeClient).mockImplementation(async (command) =>
			command === "get_app_settings" ? serverSettings : undefined,
		);
	});

	it("loads and saves daemon settings without relaying shell preferences", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(invokeTauri).not.toHaveBeenCalled();
		act(() => result.current.setDraft({ ...settings, close_to_tray: false }));
		await act(() => result.current.save());
		expect(invokeClient).toHaveBeenCalledWith(
			"update_app_settings",
			{
				app: { ...settings, close_to_tray: false },
			},
			expect.anything(),
		);
		expect(invokeTauri).not.toHaveBeenCalled();
	});

	it("reports autostart failures without relaying window preferences", async () => {
		vi.mocked(isEnabled).mockRejectedValueOnce(
			new Error("autostart unavailable"),
		);
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() =>
			expect(result.current.error).toBe("autostart unavailable"),
		);
		expect(invokeTauri).not.toHaveBeenCalled();
	});

	it("keeps shell preferences when daemon rejects the save", async () => {
		const { result } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		vi.mocked(invokeClient).mockRejectedValueOnce(new Error("save failed"));
		act(() => result.current.setDraft({ ...settings, close_to_tray: false }));
		await act(async () => {
			await expect(result.current.save()).rejects.toThrow("save failed");
		});
		expect(invokeTauri).not.toHaveBeenCalled();
		expect(result.current.isDirty).toBe(true);
	});

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
		expect(invokeTauri).not.toHaveBeenCalled();
		expect(result.current.draft).toEqual({
			...settings,
			start_minimized: false,
		});
	});
});
