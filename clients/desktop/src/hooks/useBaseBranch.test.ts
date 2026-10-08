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
	const notice = vi.fn();
	window.addEventListener("releash-client-error", notice);
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
	expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
		"save failed",
	);
	window.removeEventListener("releash-client-error", notice);
	log.mockRestore();
});

it("baseと選択肢の購読失敗を区別し回復後の値を表示する", () => {
	vi.mocked(subscribeState).mockImplementation(() => vi.fn());
	const { result } = renderHook(() => useBaseBranch("/repo", "feature"));
	const [base, branches] = vi.mocked(subscribeState).mock.calls;
	act(() => {
		base[1]("main");
		branches[1]([{ name: "main", is_remote: false }]);
	});
	act(() => base[2](new Error("base denied")));
	expect(result.current.baseBranch).toBe("main");
	expect(result.current.error).toBe("base denied");
	act(() => {
		base[1]("develop");
		branches[2](new Error("branches denied"));
	});
	expect(result.current.localBranches).toEqual(["main"]);
	expect(result.current.error).toBe("branches denied");
	act(() => branches[1]([{ name: "develop", is_remote: false }]));
	expect(result.current.error).toBeNull();
	expect(result.current.baseBranch).toBe("develop");
	expect(result.current.localBranches).toEqual(["develop"]);
});
