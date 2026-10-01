import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeClient, subscribeState } from "@/lib/client";
import { useIssues } from "./useIssues";

vi.mock("@/lib/client", () => ({
	invokeClient: vi.fn(),
	subscribeState: vi.fn(),
}));
beforeEach(() => vi.clearAllMocks());
it("issueは購読から受け取り手動更新は結果を返さない操作だけを要求する", async () => {
	let receive!: Parameters<typeof subscribeState>[1];
	const release = vi.fn();
	vi.mocked(subscribeState).mockImplementation((_target, callback) => {
		receive = callback;
		return release;
	});
	const { result, unmount } = renderHook(() => useIssues("/repo"));
	expect(result.current.loading).toBe(true);
	act(() => receive([]));
	expect(result.current.loading).toBe(false);
	vi.mocked(invokeClient).mockResolvedValueOnce(undefined);
	await act(async () => {
		await result.current.refresh();
	});
	expect(invokeClient).toHaveBeenCalledExactlyOnceWith("fetch_issues", {
		repoPath: "/repo",
	});
	expect(result.current.issues).toEqual([]);
	unmount();
	expect(release).toHaveBeenCalledOnce();
});

it("手動更新の失敗通知で直前の一覧を無効にし回復を待つ", async () => {
	const issues = [
		{
			number: 1,
			title: "Issue",
			state: "OPEN",
			url: "https://example.test/1",
			author: { login: "author" },
			created_at: "",
			updated_at: "",
			labels: [],
			assignees: [],
			body: "",
			milestone: null,
			default_branch_name: "issue-1",
		},
	];
	vi.mocked(subscribeState).mockImplementation((_target, receive) => {
		receive(issues);
		return vi.fn();
	});
	const failure = new Error("offline");
	const notice = vi.fn();
	window.addEventListener("releash-client-error", notice);
	vi.mocked(invokeClient).mockRejectedValueOnce(failure);
	const log = vi.spyOn(console, "error").mockImplementation(() => {});
	try {
		const { result } = renderHook(() => useIssues("/repo"));
		await act(async () => {
			await expect(result.current.refresh()).resolves.toBeUndefined();
			vi.mocked(subscribeState).mock.calls[0][2](failure);
		});
		expect(result.current.issues).toEqual([]);
		expect(result.current.error).toBe("offline");
		expect(result.current.loading).toBe(false);
		expect(log).toHaveBeenCalledWith("Failed to fetch issues:", failure);
		expect((notice.mock.calls[0][0] as CustomEvent<string>).detail).toBe(
			"offline",
		);
		expect(invokeClient).toHaveBeenCalledExactlyOnceWith("fetch_issues", {
			repoPath: "/repo",
		});
		expect(subscribeState).toHaveBeenCalledTimes(1);
	} finally {
		window.removeEventListener("releash-client-error", notice);
		log.mockRestore();
	}
});

it("購読の読取失敗を表示し回復を待つ", () => {
	vi.mocked(subscribeState).mockImplementation(() => vi.fn());
	const { result } = renderHook(() => useIssues("/repo"));
	const [, receive, fail] = vi.mocked(subscribeState).mock.calls[0];
	act(() => receive([]));
	act(() => fail(new Error("issues denied")));
	expect(result.current.error).toBe("issues denied");
	expect(result.current.loading).toBe(false);
	expect(result.current.issues).toEqual([]);
	act(() => receive([]));
	expect(result.current.error).toBeNull();
});
