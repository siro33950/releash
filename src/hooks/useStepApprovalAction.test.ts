import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStepApprovalAction } from "./useStepApprovalAction";

const mockInvoke = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("useStepApprovalAction", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		mockInvoke.mockResolvedValue(undefined);
	});

	it("does not submit reject when comment is blank", async () => {
		const { result } = renderHook(() =>
			useStepApprovalAction({
				worktreePath: "/repo",
				executionId: "exec-001",
				stepName: "review",
			}),
		);

		act(() => {
			result.current.setRejectComment("   ");
		});

		expect(result.current.canSubmitReject).toBe(false);

		await act(async () => {
			await result.current.submitReject();
		});

		expect(mockInvoke).not.toHaveBeenCalled();
	});
});
