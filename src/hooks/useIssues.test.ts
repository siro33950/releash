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

it("手動更新の失敗を処理し直前の一覧を保持して再取得しない", async () => {
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
	vi.mocked(invokeClient).mockRejectedValueOnce(failure);
	const log = vi.spyOn(console, "error").mockImplementation(() => {});
	try {
		const { result } = renderHook(() => useIssues("/repo"));
		await act(async () => {
			await expect(result.current.refresh()).resolves.toBeUndefined();
		});
		expect(result.current.issues).toEqual(issues);
		expect(result.current.loading).toBe(false);
		expect(log).toHaveBeenCalledWith("Failed to fetch issues:", failure);
		expect(invokeClient).toHaveBeenCalledExactlyOnceWith("fetch_issues", {
			repoPath: "/repo",
		});
		expect(subscribeState).toHaveBeenCalledTimes(1);
	} finally {
		log.mockRestore();
	}
});
