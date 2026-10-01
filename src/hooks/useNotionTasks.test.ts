import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeClient, subscribeState } from "@/lib/client";
import { useNotionTasks } from "./useNotionTasks";

beforeEach(() => {
	vi.clearAllMocks();
	vi.mocked(subscribeState).mockImplementation(() => vi.fn());
});
const task = {
	id: "1",
	title: "Task",
	url: "",
	labels: {},
	branch_name: "feat/task",
	created_at: "",
	last_edited_at: "",
};

it("絞り込みなしはrepoと件数だけを購読し単発取得しない", () => {
	const { result, unmount } = renderHook(() => useNotionTasks("/repo"));
	expect(subscribeState).toHaveBeenCalledWith(
		{ kind: "notion-tasks", args: ["/repo", "20"] },
		expect.any(Function),
		expect.any(Function),
	);
	expect(result.current.loading).toBe(true);
	act(() =>
		vi
			.mocked(subscribeState)
			.mock.calls[0][1]({ page: { tasks: [task], has_more: true } }),
	);
	expect(result.current.tasks).toEqual([task]);
	expect(result.current.loading).toBe(false);
	expect(invokeClient).not.toHaveBeenCalled();
	const release = vi.mocked(subscribeState).mock.results[0].value;
	unmount();
	expect(release).toHaveBeenCalledOnce();
});

it("Load moreは20件増やして一覧を置き換え届くまでは前の一覧を表示する", () => {
	const { result } = renderHook(() => useNotionTasks("/repo"));
	act(() =>
		vi
			.mocked(subscribeState)
			.mock.calls[0][1]({ page: { tasks: [task], has_more: true } }),
	);
	act(() => result.current.loadMore());
	expect(subscribeState).toHaveBeenLastCalledWith(
		{ kind: "notion-tasks", args: ["/repo", "40"] },
		expect.any(Function),
		expect.any(Function),
	);
	expect(result.current.tasks).toEqual([task]);
	expect(result.current.loading).toBe(true);
	act(() => result.current.loadMore());
	expect(subscribeState).toHaveBeenCalledTimes(2);
	const tasks = [task, { ...task, id: "2" }];
	act(() =>
		vi
			.mocked(subscribeState)
			.mock.calls[1][1]({ page: { tasks, has_more: false } }),
	);
	expect(result.current.tasks).toEqual(tasks);
	expect(result.current.hasMore).toBe(false);
});

it("検索はdebounceして件数を戻しラベル順を保持し空の絞り込みを省く", () => {
	vi.useFakeTimers();
	try {
		const { result } = renderHook(() => useNotionTasks("/repo"));
		act(() => result.current.search("old", {}));
		act(() =>
			result.current.search("query", {
				Tags: ["z", "a"],
				Status: [],
				Assignee: ["Bob"],
			}),
		);
		expect(subscribeState).toHaveBeenCalledTimes(1);
		act(() => vi.advanceTimersByTime(300));
		expect(subscribeState).toHaveBeenLastCalledWith(
			{
				kind: "notion-tasks",
				args: [
					"/repo",
					"20",
					"title=query",
					'labels={"Tags":["z","a"],"Assignee":["Bob"]}',
				],
			},
			expect.any(Function),
			expect.any(Function),
		);
		act(() => result.current.search("", { Tags: [] }));
		act(() => vi.advanceTimersByTime(300));
		expect(subscribeState).toHaveBeenLastCalledWith(
			{ kind: "notion-tasks", args: ["/repo", "20"] },
			expect.any(Function),
			expect.any(Function),
		);
	} finally {
		vi.useRealTimers();
	}
});

it("最後の一覧と分類された失敗を受け取り回復時に失敗を消す", () => {
	const { result } = renderHook(() => useNotionTasks("/repo"));
	const [, receive, fail] = vi.mocked(subscribeState).mock.calls[0];
	const page = { tasks: [task], has_more: false };
	act(() =>
		receive({
			page,
			readError: {
				code: 9,
				message: "Notion設定が見つかりません",
				configMissing: true,
			},
		}),
	);
	expect(result.current.tasks).toEqual([task]);
	expect(result.current.readError?.code).toBe(9);
	expect(result.current.readError?.configMissing).toBe(true);
	expect(result.current.error).toBe("Notion設定が見つかりません");
	act(() => fail(new Error("stream failed")));
	expect(result.current.tasks).toEqual([task]);
	expect(result.current.error).toBe("stream failed");
	act(() => receive({ page }));
	expect(result.current.error).toBeNull();
});

it("repo変更では旧一覧を表示せず検索タイマーを破棄する", () => {
	vi.useFakeTimers();
	try {
		const { result, rerender } = renderHook(
			({ repo }) => useNotionTasks(repo),
			{ initialProps: { repo: "/repo" } },
		);
		act(() =>
			vi
				.mocked(subscribeState)
				.mock.calls[0][1]({ page: { tasks: [task], has_more: true } }),
		);
		act(() => result.current.search("old", {}));
		rerender({ repo: "/other" });
		act(() => vi.advanceTimersByTime(300));
		expect(result.current.tasks).toEqual([]);
		expect(subscribeState).toHaveBeenLastCalledWith(
			{ kind: "notion-tasks", args: ["/other", "20"] },
			expect.any(Function),
			expect.any(Function),
		);
	} finally {
		vi.useRealTimers();
	}
});
