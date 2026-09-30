import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUpdateChecker } from "../useUpdateChecker";

const mockCheck = vi.mocked(invoke);

describe("useUpdateChecker", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockCheck.mockResolvedValue(null);
	});

	it("should not check when enabled=false", () => {
		renderHook(() => useUpdateChecker(false));
		expect(mockCheck).not.toHaveBeenCalled();
	});

	it("should check once on mount when enabled=true", async () => {
		mockCheck.mockResolvedValue(null);
		const { result } = renderHook(() => useUpdateChecker(true));

		expect(mockCheck).toHaveBeenCalledTimes(1);
		await waitFor(() => {
			expect(result.current.status).toBe("idle");
		});
	});

	it("should set status to idle when no update available", async () => {
		mockCheck.mockResolvedValue(null);
		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("idle");
		});
		expect(result.current.updateInfo).toBeNull();
	});

	it("should set status to available when update found", async () => {
		const mockUpdate = {
			version: "1.2.0",
			notes: "Bug fixes and improvements",
			date: "2025-01-01",
			downloadAndInstall: vi.fn(),
		};
		mockCheck.mockResolvedValue(mockUpdate);

		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("available");
		});
		expect(result.current.updateInfo).toEqual({
			version: "1.2.0",
			notes: "Bug fixes and improvements",
		});
	});

	it("should silently return to idle on check failure", async () => {
		mockCheck.mockRejectedValue(new Error("Network error"));
		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("idle");
		});
		expect(result.current.error).toBeNull();
	});

	it("should re-check when enabled changes from false to true", async () => {
		mockCheck.mockResolvedValue(null);
		const { result, rerender } = renderHook(
			({ enabled }) => useUpdateChecker(enabled),
			{ initialProps: { enabled: false } },
		);

		expect(mockCheck).not.toHaveBeenCalled();

		rerender({ enabled: true });

		expect(mockCheck).toHaveBeenCalledTimes(1);
		await waitFor(() => {
			expect(result.current.status).toBe("idle");
		});
	});

	it("should return to idle on dismiss", async () => {
		const mockUpdate = {
			version: "1.2.0",
			notes: "Notes",
			date: "2025-01-01",
			downloadAndInstall: vi.fn(),
		};
		mockCheck.mockResolvedValue(mockUpdate);

		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("available");
		});

		act(() => {
			result.current.dismiss();
		});

		expect(result.current.status).toBe("idle");
		expect(result.current.updateInfo).toBeNull();
	});

	it("should download, install and relaunch", async () => {
		const mockDownloadAndInstall = vi.fn().mockResolvedValue(undefined);
		const mockUpdate = {
			version: "1.2.0",
			notes: "Notes",
			date: "2025-01-01",
			downloadAndInstall: mockDownloadAndInstall,
		};
		mockCheck.mockResolvedValue(mockUpdate);

		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("available");
		});

		act(() => {
			result.current.downloadAndInstall();
		});

		expect(result.current.status).toBe("downloading");

		await waitFor(() => {
			expect(invoke).toHaveBeenCalledWith("install_desktop_update");
		});
	});
});

it.each([true, false])(
	"StrictModeでも初回確認は有効=%sの一度だけで結果を反映する",
	async (enabled) => {
		vi.mocked(invoke).mockClear();
		let complete!: (value: { version: string; notes: string }) => void;
		vi.mocked(invoke).mockImplementation(
			() =>
				new Promise((resolve) => {
					complete = resolve;
				}),
		);
		const { result, rerender } = renderHook(() => useUpdateChecker(enabled), {
			wrapper: StrictMode,
		});
		expect(invoke).toHaveBeenCalledTimes(enabled ? 1 : 0);
		if (enabled) {
			await act(async () => complete({ version: "2.0.0", notes: "Update" }));
			expect(result.current.status).toBe("available");
			expect(result.current.updateInfo).toEqual({
				version: "2.0.0",
				notes: "Update",
			});
		}
		rerender();
		expect(invoke).toHaveBeenCalledTimes(enabled ? 1 : 0);
	},
);
