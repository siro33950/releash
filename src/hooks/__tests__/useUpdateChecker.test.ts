import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUpdateChecker } from "../useUpdateChecker";

const mockCheck = vi.mocked(check);
const mockRelaunch = vi.mocked(relaunch);

describe("useUpdateChecker", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockCheck.mockResolvedValue(null);
		mockRelaunch.mockResolvedValue(undefined);
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
			body: "Bug fixes and improvements",
			date: "2025-01-01",
			downloadAndInstall: vi.fn(),
		};
		mockCheck.mockResolvedValue(
			mockUpdate as unknown as Awaited<ReturnType<typeof check>>,
		);

		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("available");
		});
		expect(result.current.updateInfo).toEqual({
			version: "1.2.0",
			notes: "Bug fixes and improvements",
		});
	});

	it("should set status to error on check failure", async () => {
		mockCheck.mockRejectedValue(new Error("Network error"));
		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("error");
		});
		expect(result.current.error).toBe("Network error");
	});

	it("should return to idle on dismiss", async () => {
		const mockUpdate = {
			version: "1.2.0",
			body: "Notes",
			date: "2025-01-01",
			downloadAndInstall: vi.fn(),
		};
		mockCheck.mockResolvedValue(
			mockUpdate as unknown as Awaited<ReturnType<typeof check>>,
		);

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
			body: "Notes",
			date: "2025-01-01",
			downloadAndInstall: mockDownloadAndInstall,
		};
		mockCheck.mockResolvedValue(
			mockUpdate as unknown as Awaited<ReturnType<typeof check>>,
		);

		const { result } = renderHook(() => useUpdateChecker(true));

		await waitFor(() => {
			expect(result.current.status).toBe("available");
		});

		act(() => {
			result.current.downloadAndInstall();
		});

		expect(result.current.status).toBe("downloading");

		await waitFor(() => {
			expect(mockDownloadAndInstall).toHaveBeenCalled();
		});

		await waitFor(() => {
			expect(mockRelaunch).toHaveBeenCalled();
		});
	});
});
