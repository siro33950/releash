import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useProviderAvailabilitySettings } from "./useProviderAvailabilitySettings";

const mockInvoke = vi.mocked(invoke);

describe("useProviderAvailabilitySettings", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
	});

	it.each([
		[
			{ code: "PROVIDER_AVAILABILITY_CORRUPT", message: "backend message" },
			"backend message",
		],
		["plain message", "plain message"],
	])(
		"Provider executable設定の取得失敗からmessageだけを保持する",
		async (rejection, expected) => {
			mockInvoke.mockRejectedValueOnce(rejection);

			const { result } = renderHook(() =>
				useProviderAvailabilitySettings(true),
			);

			await waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith("get_provider_availability");
				expect(result.current.loading).toBe(false);
				expect(result.current.error).toBe(expected);
			});
		},
	);
});
