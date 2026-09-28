import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { stateSubscriptions } from "@/test/stateSubscriptions";
import { useProviderAvailabilitySettings } from "./useProviderAvailabilitySettings";

const states = stateSubscriptions();
vi.mock("@/lib/client", async (importOriginal) => ({
	...(await importOriginal<typeof import("@/lib/client")>()),
	invokeClient: vi.fn(),
	subscribeState: (...args: Parameters<typeof states.subscribeState>) =>
		states.subscribeState(...args),
}));

describe("useProviderAvailabilitySettings", () => {
	beforeEach(() => {
		states.clear();
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
			const { result } = renderHook(() =>
				useProviderAvailabilitySettings(true),
			);
			expect(result.current.loading).toBe(true);
			act(() => states.fail("provider-availability", rejection));

			await waitFor(() => {
				expect(result.current.loading).toBe(false);
				expect(result.current.error).toBe(expected);
			});
			expect(states.subscribeState).toHaveBeenCalledWith(
				"provider-availability",
				expect.any(Function),
				expect.any(Function),
			);
		},
	);

	it("閉じている間は購読せず開くと届いた一覧を表示する", async () => {
		const provider = {
			provider: "claude",
			displayName: "Claude",
			defaultExecutable: "claude",
			configuredExecutable: null,
			effectiveExecutable: "claude",
			available: true,
			resolvedExecutable: "/usr/bin/claude",
			unavailableReason: null,
		};
		states.publish("provider-availability", { providers: [provider] });
		const { result, rerender } = renderHook(
			({ open }) => useProviderAvailabilitySettings(open),
			{ initialProps: { open: false } },
		);
		expect(states.subscribeState).not.toHaveBeenCalled();
		expect(result.current.loading).toBe(false);
		rerender({ open: true });
		await waitFor(() => expect(result.current.providers).toEqual([provider]));
		expect(result.current.drafts).toEqual({ claude: "" });
	});
});
