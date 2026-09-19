import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient } from "@/lib/client";
import { useCurrentBranch } from "./useCurrentBranch";

vi.mock("@/lib/client", () => ({ invokeClient: vi.fn() }));

describe("useCurrentBranch", () => {
	beforeEach(() => vi.clearAllMocks());
	it("wsで取得したブランチを表示し失敗時は非表示にする", async () => {
		vi.mocked(invokeClient).mockResolvedValueOnce("ws-branch");
		const { result } = renderHook(() => useCurrentBranch("/repo"));
		await waitFor(() => expect(result.current.branch).toBe("ws-branch"));
		expect(invokeClient).toHaveBeenCalledWith("get_current_branch", {
			repoPath: "/repo",
		});
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		vi.mocked(invokeClient).mockRejectedValueOnce(new Error("disconnected"));
		await act(() => result.current.refresh());
		expect(result.current.branch).toBeNull();
		expect(invoke).not.toHaveBeenCalled();
		log.mockRestore();
	});
	it("リポジトリ未選択では取得しない", () => {
		const { result } = renderHook(() => useCurrentBranch(null));
		expect(result.current.branch).toBeNull();
		expect(invokeClient).not.toHaveBeenCalled();
	});
});
