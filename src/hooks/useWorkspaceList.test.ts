import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { workspaceListSnapshot } from "@/test/workspaceList";
import { useWorkspaceList } from "./useWorkspaceList";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(),
	watch: vi.fn(),
	stop: vi.fn(),
	agent: vi.fn(),
}));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	listenClient: mocks.listen,
	watchClient: mocks.watch,
}));
vi.mock("@/lib/agentSessionEvents", () => ({
	subscribeAgentSessionChanged: mocks.agent,
}));

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("useWorkspaceList", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mocks.invoke.mockResolvedValue(workspaceListSnapshot());
		mocks.listen.mockResolvedValue(vi.fn());
		mocks.watch.mockReturnValue(mocks.stop);
		mocks.agent.mockReturnValue(vi.fn());
	});
	afterEach(() => vi.useRealTimers());

	it("無関係な再描画では一覧モデルの参照を維持する", async () => {
		const { result, rerender } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(result.current.snapshot).not.toBeNull());
		const previous = result.current;
		rerender();
		expect(result.current).toBe(previous);
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
	});

	it("一覧とエラーの変更はモデルに反映し更新操作の参照を維持する", async () => {
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(result.current.snapshot).not.toBeNull());
		const previous = result.current;
		mocks.invoke.mockRejectedValueOnce(new Error("deadline exceeded"));
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current).not.toBe(previous);
		expect(result.current.snapshot).toBe(previous.snapshot);
		expect(result.current.error).toBe("deadline exceeded");
		const failed = result.current;
		mocks.invoke.mockResolvedValueOnce(previous.snapshot);
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current).not.toBe(failed);
		expect(result.current.error).toBeNull();
		const recovered = result.current;
		const latest = workspaceListSnapshot();
		latest.generation = 2;
		mocks.invoke.mockResolvedValueOnce(latest);
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current).not.toBe(recovered);
		expect(result.current.snapshot).toBe(latest);
		for (const model of [failed, recovered, result.current]) {
			expect(model.refresh).toBe(previous.refresh);
			expect(model.refreshWorktree).toBe(previous.refreshWorktree);
			expect(model.refreshRepository).toBe(previous.refreshRepository);
		}
	});

	it.each(["worktree", "repository", "all"])(
		"WorktreeのRPC失敗は無関係な成功で消えず%sの再取得成功で解消する",
		async (scope) => {
			const { result } = renderHook(() => useWorkspaceList());
			await waitFor(() => expect(result.current.snapshot).not.toBeNull());
			const previous = result.current.snapshot;
			mocks.invoke.mockRejectedValueOnce(new Error("offline"));
			await act(async () => {
				await result.current.refreshWorktree("/repo");
			});
			expect(result.current.snapshot).toBe(previous);
			expect(result.current.error).toBeNull();
			expect(result.current.worktreeErrors["/repo"]).toBe("offline");
			await act(async () => {
				await result.current.refreshWorktree("/other");
			});
			await act(async () => {
				await result.current.refreshRepository("/other");
			});
			expect(result.current.worktreeErrors["/repo"]).toBe("offline");
			const failed = workspaceListSnapshot();
			failed.repositories[0].worktrees[0].status.error = "still offline";
			mocks.invoke.mockResolvedValueOnce(failed);
			await act(async () => {
				await result.current.refresh();
			});
			expect(result.current.worktreeErrors["/repo"]).toBe("offline");
			await act(async () => {
				if (scope === "worktree") await result.current.refreshWorktree("/repo");
				else if (scope === "repository")
					await result.current.refreshRepository("/repo");
				else await result.current.refresh();
			});
			expect(result.current.worktreeErrors).toEqual({});
		},
	);

	it.each(["repository", "all"])(
		"RepositoryのRPC失敗は対象だけに保持し%sの再取得成功で解消する",
		async (scope) => {
			const { result } = renderHook(() => useWorkspaceList());
			await waitFor(() => expect(result.current.snapshot).not.toBeNull());
			const previous = result.current.snapshot;
			mocks.invoke.mockRejectedValueOnce(new Error("repository offline"));
			await act(async () => {
				await result.current.refreshRepository("/repo");
			});
			expect(result.current.snapshot).toBe(previous);
			expect(result.current.error).toBeNull();
			expect(result.current.repositoryErrors).toEqual({
				"/repo": "repository offline",
			});
			await act(async () => {
				await result.current.refreshRepository("/other");
				await result.current.refreshWorktree("/repo");
			});
			expect(result.current.repositoryErrors["/repo"]).toBe(
				"repository offline",
			);
			for (const status of [
				{ loaded: false, error: null },
				{ loaded: true, error: "scan failed" },
			]) {
				const failed = workspaceListSnapshot();
				failed.repositories[0].status = status;
				mocks.invoke.mockResolvedValueOnce(failed);
				await act(async () => {
					await result.current.refresh();
				});
				expect(result.current.repositoryErrors["/repo"]).toBe(
					"repository offline",
				);
			}
			await act(async () => {
				if (scope === "repository")
					await result.current.refreshRepository("/repo");
				else await result.current.refresh();
			});
			expect(result.current.repositoryErrors).toEqual({});
		},
	);

	it("全体RPC失敗は局所成功や登録一覧の取得失敗では消えず全体の再取得成功で解消する", async () => {
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(result.current.snapshot).not.toBeNull());
		mocks.invoke.mockRejectedValueOnce(new Error("all offline"));
		await act(async () => {
			await result.current.refresh();
		});
		for (const refresh of [
			() => result.current.refreshWorktree("/repo"),
			() => result.current.refreshRepository("/repo"),
		]) {
			await act(async () => {
				await refresh();
			});
			expect(result.current.error).toBe("all offline");
		}
		for (const status of [
			{ loaded: false, error: null },
			{ loaded: true, error: "repositories failed" },
		]) {
			const failed = workspaceListSnapshot();
			failed.status = status;
			mocks.invoke.mockResolvedValueOnce(failed);
			await act(async () => {
				await result.current.refresh();
			});
			expect(result.current.error).toBe("all offline");
		}
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current.error).toBeNull();
	});

	it("初回未取得と正常な空を区別する", async () => {
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValue(pending.promise);
		const { result } = renderHook(() => useWorkspaceList());
		expect(result.current.snapshot).toBeNull();
		await act(async () =>
			pending.resolve({
				generation: 1,
				status: { loaded: true, error: null },
				repositories: [],
			}),
		);
		expect(result.current.snapshot?.status.loaded).toBe(true);
		expect(result.current.snapshot?.repositories).toEqual([]);
	});

	it("手動更新中も失敗後も前回の全階層の一覧を保持し再試行できる", async () => {
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(result.current.snapshot).not.toBeNull());
		const previous = result.current.snapshot;
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(pending.promise);
		let request!: Promise<unknown>;
		act(() => {
			request = result.current.refresh();
		});
		expect(result.current.snapshot).toBe(previous);
		await act(async () => {
			pending.reject(new Error("deadline exceeded"));
			await request;
		});
		expect(result.current.snapshot).toBe(previous);
		expect(result.current.error).toBe("deadline exceeded");
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current.error).toBeNull();
	});

	it("一覧RPCを直列に呼びRustから受け取った結果を順に表示する", async () => {
		const old = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(old.promise);
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));
		const latest = workspaceListSnapshot();
		latest.generation = 2;
		latest.repositories[0].branches[0].name = "latest";
		mocks.invoke.mockResolvedValueOnce(latest);
		let next!: Promise<unknown>;
		act(() => {
			next = result.current.refresh();
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		await act(async () => {
			old.resolve(workspaceListSnapshot());
			await next;
		});
		expect(result.current.snapshot).toBe(latest);
	});

	it("登録済みRepositoryを監視し削除とunmountで解除する", async () => {
		const { result, unmount } = renderHook(() => useWorkspaceList());
		await waitFor(() =>
			expect(mocks.watch).toHaveBeenCalledWith(
				"start_git_dir_watching",
				{ repoPath: "/repo" },
				expect.any(Function),
			),
		);
		mocks.invoke.mockResolvedValueOnce({
			generation: 2,
			status: { loaded: true, error: null },
			repositories: [],
		});
		await act(async () => {
			await result.current.refresh();
		});
		expect(mocks.stop).toHaveBeenCalledTimes(1);
		unmount();
	});

	it("局所再読込も同じRPC列を使い対象worktreeを指定する", async () => {
		const pending = deferred<ReturnType<typeof workspaceListSnapshot>>();
		mocks.invoke.mockReturnValueOnce(pending.promise);
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));
		let next!: Promise<unknown>;
		act(() => {
			next = result.current.refreshWorktree("/repo");
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		await act(async () => {
			pending.reject(new Error("deadline exceeded"));
			await next;
		});
		expect(mocks.invoke).toHaveBeenLastCalledWith("refresh_workspaces", {
			worktreePath: "/repo",
		});
		expect(result.current.snapshot).not.toBeNull();
		expect(result.current.error).toBe("deadline exceeded");
	});

	it("Repository再読込は対象をRPCへ渡す", async () => {
		const { result } = renderHook(() => useWorkspaceList());
		await waitFor(() => expect(result.current.snapshot).not.toBeNull());
		await act(async () => {
			await result.current.refreshRepository("/repo");
		});
		expect(mocks.invoke).toHaveBeenLastCalledWith("refresh_workspaces", {
			worktreePath: undefined,
			repoPath: "/repo",
		});
	});

	it("120秒周期は可視時だけ全体を再取得する", async () => {
		vi.useFakeTimers();
		const { result } = renderHook(() => useWorkspaceList());
		await act(async () => {});
		expect(result.current.snapshot).not.toBeNull();
		await act(async () => vi.advanceTimersByTimeAsync(119_999));
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		await act(async () => vi.advanceTimersByTimeAsync(1));
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		const visibility = vi
			.spyOn(document, "visibilityState", "get")
			.mockReturnValue("hidden");
		await act(async () => vi.advanceTimersByTimeAsync(120_000));
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		visibility.mockRestore();
	});

	it("branchとRepositoryの通知は同じ全体更新を呼ぶ", async () => {
		vi.useFakeTimers();
		renderHook(() => useWorkspaceList());
		await act(async () => {});
		for (const name of ["branch-list-sync", "repo-paths-changed"]) {
			await act(async () => {
				mocks.listen.mock.calls.find(([event]) => event === name)?.[1]({
					payload: {},
				});
				await vi.advanceTimersByTimeAsync(80);
			});
		}
		expect(mocks.invoke.mock.calls).toEqual(
			Array(3).fill(["refresh_workspaces", { worktreePath: undefined }]),
		);
	});

	it.each(["workflow", "session", "workspace"])(
		"%sのパス付き通知も手動更新と同じ全体更新を要求する",
		async (source) => {
			vi.useFakeTimers();
			renderHook(() => useWorkspaceList());
			await act(async () => {});
			mocks.invoke.mockClear();
			await act(async () => {
				const detail = { worktreePath: "/repo/hidden" };
				if (source === "workflow") {
					mocks.listen.mock.calls.find(
						([event]) => event === "workflow-execution-changed",
					)?.[1]({ payload: detail });
				} else if (source === "session") {
					mocks.agent.mock.calls[0][0](detail);
				} else {
					window.dispatchEvent(
						new CustomEvent("workspace-tree-refresh", { detail }),
					);
				}
				await vi.advanceTimersByTimeAsync(80);
			});
			expect(mocks.invoke.mock.calls).toEqual([
				["refresh_workspaces", { worktreePath: undefined }],
			]);
		},
	);

	it("異なるWorktreeの連続通知も全体更新1回にまとめ解除時に予約を破棄する", async () => {
		vi.useFakeTimers();
		const { unmount } = renderHook(() => useWorkspaceList());
		await act(async () => {});
		mocks.invoke.mockClear();
		await act(async () => {
			mocks.agent.mock.calls[0][0]({ worktreePath: "/repo/a" });
			mocks.agent.mock.calls[0][0]({ worktreePath: "/repo/b" });
			mocks.agent.mock.calls[0][0]({ worktreePath: "/repo/a" });
			await vi.advanceTimersByTimeAsync(80);
		});
		expect(mocks.invoke.mock.calls).toEqual([
			["refresh_workspaces", { worktreePath: undefined }],
		]);
		act(() => mocks.agent.mock.calls[0][0]({ worktreePath: "/repo/a" }));
		unmount();
		await act(async () => vi.advanceTimersByTimeAsync(80));
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
	});

	it("再接続など対象Worktreeを持たない通知では全体を再取得する", async () => {
		vi.useFakeTimers();
		renderHook(() => useWorkspaceList());
		await act(async () => {});
		mocks.invoke.mockClear();
		await act(async () => {
			mocks.listen.mock.calls.find(
				([event]) => event === "workflow-execution-changed",
			)?.[2]();
			await vi.advanceTimersByTimeAsync(80);
			mocks.agent.mock.calls[0][0]({});
			await vi.advanceTimersByTimeAsync(80);
			window.dispatchEvent(new Event("workspace-tree-refresh"));
			await vi.advanceTimersByTimeAsync(80);
		});
		expect(mocks.invoke.mock.calls).toEqual(
			Array(3).fill(["refresh_workspaces", { worktreePath: undefined }]),
		);
	});
	it("削除中は1秒間隔で全体を再取得し、削除後は通常間隔に戻る", async () => {
		vi.useFakeTimers();
		const deleting = workspaceListSnapshot();
		deleting.repositories[0].branches[0].is_deleting = true;
		mocks.invoke.mockResolvedValue(deleting);
		const { result, unmount } = renderHook(() => useWorkspaceList());
		await act(async () => {});
		expect(result.current.snapshot).toEqual(deleting);
		const completed = workspaceListSnapshot();
		completed.repositories[0].branches = [];
		mocks.invoke.mockResolvedValue(completed);
		await act(async () => {
			await vi.advanceTimersByTimeAsync(1_000);
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		expect(result.current.snapshot).toEqual(completed);
		await act(async () => {
			await vi.advanceTimersByTimeAsync(1_000);
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		await act(async () => {
			await vi.advanceTimersByTimeAsync(119_000);
		});
		expect(mocks.invoke).toHaveBeenCalledTimes(3);
		unmount();
	});
});
