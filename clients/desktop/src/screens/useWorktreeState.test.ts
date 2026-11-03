import { renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "@/types/settings";
import { useWorktreeState } from "./useWorktreeState";

vi.mock("@/hooks/useCurrentBranch", () => ({
	useCurrentBranch: () => ({ branch: "main", error: null }),
}));

vi.mock("@/lib/client", () => ({ invokeClient: vi.fn() }));

describe("useWorktreeState", () => {
	it.each([false, true])(
		"layoutが未保存なら既定の表示状態とdiffOnlyMode設定(%s)を使う",
		(defaultDiffOnlyMode) => {
			const { result } = renderHook(() =>
				useWorktreeState({
					rootPath: "/repo",
					settings: { ...DEFAULT_SETTINGS, defaultDiffOnlyMode },
					onSettingsSave: vi.fn(),
					isActive: true,
					initialWorkspaceState: { version: 1 },
				}),
			);
			expect(result.current.rightBottomCollapsed).toBe(false);
			expect(result.current.reviewCollapsed).toBe(false);
			expect(result.current.diffOnlyMode).toBe(defaultDiffOnlyMode);
			expect(result.current.selectedDiffFile).toBeNull();
		},
	);
});
