import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeClient, subscribeState } from "@/lib/client";
import { useBaseBranch } from "./useBaseBranch";

vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
	subscribeState: vi.fn(),
}));
beforeEach(() => {
	vi.clearAllMocks();
	vi.mocked(invokeClient).mockResolvedValue(undefined);
});
it("daemonが選んだ候補とbaseを購読し設定後も購読による確定を待つ", async () => {
	vi.mocked(subscribeState).mockImplementation((target, receive) => {
		if (typeof target !== "string" && target.kind === "branches")
			receive([{ name: "main", is_remote: false }]);
		else receive("main");
		return vi.fn();
	});
	const { result } = renderHook(() => useBaseBranch("/repo", "feature"));
	expect(subscribeState).toHaveBeenCalledWith(
		{ kind: "branches", args: ["/repo", "feature"] },
		expect.any(Function),
		expect.any(Function),
	);
	expect(result.current.localBranches).toEqual(["main"]);
	await act(async () => {
		await result.current.setBaseBranch("develop");
	});
	expect(invokeClient).toHaveBeenCalledExactlyOnceWith("set_branch_base", {
		repoPath: "/repo",
		branchName: "feature",
		base: "develop",
	});
	expect(result.current.baseBranch).toBe("main");
});

it("設定失敗を処理して直前値を保持し再取得しない", async () => {
	vi.mocked(subscribeState).mockImplementation((_target, receive) => {
		if (typeof _target !== "string" && _target.kind === "branches") receive([]);
		else receive("main");
		return vi.fn();
	});
	const failure = new Error("save failed");
	vi.mocked(invokeClient).mockRejectedValueOnce(failure);
	const log = vi.spyOn(console, "error").mockImplementation(() => {});
	const { result } = renderHook(() => useBaseBranch("/repo", "feature"));
	await act(async () => {
		await expect(
			result.current.setBaseBranch("develop"),
		).resolves.toBeUndefined();
	});
	expect(result.current.baseBranch).toBe("main");
	expect(invokeClient).toHaveBeenCalledTimes(1);
	expect(subscribeState).toHaveBeenCalledTimes(2);
	expect(log).toHaveBeenCalledWith("Failed to set base branch:", failure);
	log.mockRestore();
});
