import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PrStatus, WorktreeBranch } from "@/types/git";
import { useWorktreeList } from "./useWorktreeList";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@/lib/client", async () => ({
	watchClient: (await import("@/test/watchClient")).mockWatchClient((...args) =>
		mockInvoke(...args),
	),
	listenClient: (...args: unknown[]) => mockListen(...args),
	invokeClient: (...args: unknown[]) => mockInvoke(...args),
}));

const makeBranch = (
	overrides: Partial<WorktreeBranch> = {},
): WorktreeBranch => ({
	name: "feat/test",
	worktree_path: "/tmp/wt",
	is_main_worktree: false,
	is_deleting: false,
	is_merged: false,
	has_upstream: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	base_ahead: 0,
	dirty_count: 0,
	...overrides,
});

// backend が返す表示グループを模す（振り分けは Rust 側の read model が確定する）。
function displayGroups(branches: WorktreeBranch[]) {
	return { working_areas: branches.filter((branch) => branch.worktree_path) };
}

function setupMockInvoke(
	branches: WorktreeBranch[],
	prStatus: Promise<PrStatus> = Promise.resolve({
		open_prs: {},
		merged_branches: [],
	}),
) {
	mockInvoke.mockImplementation((cmd: string) => {
		if (cmd === "start_git_dir_watching") return Promise.resolve(42);
		if (cmd === "stop_watching") return Promise.resolve();
		if (cmd === "list_branches_with_status_snapshot")
			return Promise.resolve({
				version: 1,
				stale: false,
				loading: false,
				branches,
				worktree_display_groups: displayGroups(branches),
			});
		if (cmd === "get_cached_pr_status") return prStatus;
		return Promise.resolve([]);
	});
}

describe("useWorktreeList", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		setupMockInvoke([]);
	});

	it("should start git dir watcher on mount", async () => {
		renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});
	});

	it("should stop watcher on unmount", async () => {
		const { unmount } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo",
			});
		});

		unmount();

		expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
			watcherId: 42,
		});
	});

	it("should restart watcher when repoPath changes", async () => {
		const { rerender } = renderHook(
			({ repoPath }: { repoPath: string }) => useWorktreeList(repoPath),
			{ initialProps: { repoPath: "/test/repo-a" } },
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo-a",
			});
		});

		const nextWatcherId = 99;
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching")
				return Promise.resolve(nextWatcherId);
			if (cmd === "stop_watching") return Promise.resolve();
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		rerender({ repoPath: "/test/repo-b" });

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
				watcherId: 42,
			});
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("start_git_dir_watching", {
				repoPath: "/test/repo-b",
			});
		});
	});

	it("should not crash when watcher start fails", async () => {
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching")
				return Promise.reject(new Error("repo not found"));
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(consoleSpy).toHaveBeenCalledWith(
				"Failed to start git dir watcher:",
				expect.any(Error),
			);
		});

		expect(result.current.branches).toEqual([]);
		consoleSpy.mockRestore();
	});

	it("should use 120s poll interval", async () => {
		vi.useFakeTimers();
		renderHook(() => useWorktreeList("/test/repo"));

		await act(async () => {
			await vi.waitFor(() => {
				expect(mockInvoke).toHaveBeenCalledWith(
					"list_branches_with_status_snapshot",
					{
						repoPath: "/test/repo",
					},
				);
			});
		});

		const callCountBefore = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;

		await act(async () => {
			vi.advanceTimersByTime(30_000);
		});

		const callCountAfter30s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;
		expect(callCountAfter30s).toBe(callCountBefore);

		await act(async () => {
			vi.advanceTimersByTime(90_000);
		});

		const callCountAfter120s = mockInvoke.mock.calls.filter(
			(c) => c[0] === "list_branches_with_status_snapshot",
		).length;
		expect(callCountAfter120s).toBe(callCountBefore + 1);

		vi.useRealTimers();
	});

	it("should stop watcher if unmounted before watcher start resolves", async () => {
		let resolveStart: (id: number) => void = () => {};
		mockInvoke.mockImplementation((cmd: string) => {
			if (cmd === "start_git_dir_watching") {
				return new Promise<number>((resolve) => {
					resolveStart = resolve;
				});
			}
			if (cmd === "stop_watching") return Promise.resolve();
			if (cmd === "list_branches_with_status_snapshot")
				return Promise.resolve({
					version: 1,
					stale: false,
					loading: false,
					branches: [],
				});
			if (cmd === "get_cached_pr_status")
				return Promise.resolve({ open_prs: {}, merged_branches: [] });
			return Promise.resolve();
		});

		const { unmount } = renderHook(() => useWorktreeList("/test/repo"));
		unmount();

		await act(async () => {
			resolveStart(77);
		});

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("stop_watching", {
				watcherId: 77,
			});
		});
	});

	it("should set loading to true on initial load then false after fetch", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		expect(result.current.loading).toBe(true);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.branches).toHaveLength(1);
		expect(result.current.branches[0].name).toBe("feat/test");
	});

	it("should not set loading to true when refresh is called with silent: true", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data so refresh triggers a state update
		const updatedBranch = makeBranch({ dirty_count: 5 });
		setupMockInvoke([updatedBranch]);

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		// loading should never have become true during silent refresh
		expect(result.current.loading).toBe(false);
		expect(result.current.branches[0].dirty_count).toBe(5);
	});

	it.each(["success", "failure"])(
		"PR取得が保留中でも削除中の一覧を表示し、%s後も保持する",
		async (outcome) => {
			let resolvePr!: (status: PrStatus) => void;
			let rejectPr!: (error: Error) => void;
			const prStatus = new Promise<PrStatus>((resolve, reject) => {
				resolvePr = resolve;
				rejectPr = reject;
			});
			const branch = makeBranch({ is_deleting: true });
			setupMockInvoke([branch], prStatus);
			const { result } = renderHook(() => useWorktreeList("/test/repo"));

			await waitFor(() => {
				expect(result.current.branches).toEqual([branch]);
				expect(result.current.loading).toBe(false);
			});

			await act(async () => {
				if (outcome === "success") {
					resolvePr({
						open_prs: {
							[branch.name]: {
								number: 1845,
								url: "https://example.com/pr/1845",
							},
						},
						merged_branches: [],
					});
				} else {
					rejectPr(new Error("PR lookup failed"));
				}
			});

			expect(result.current.branches).toEqual([
				outcome === "success"
					? {
							...branch,
							has_pr: true,
							pr_number: 1845,
							pr_url: "https://example.com/pr/1845",
						}
					: branch,
			]);
			expect(result.current.loading).toBe(false);
		},
	);

	it.each(["refresh", "branch-list-sync", "branch-list-refresh"])(
		"%sで取得した削除中の一覧をPR取得より先に反映し、古いPR応答で戻さない",
		async (trigger) => {
			let resolveOldPr!: (status: PrStatus) => void;
			setupMockInvoke(
				[makeBranch()],
				new Promise<PrStatus>((resolve) => {
					resolveOldPr = resolve;
				}),
			);
			const { result } = renderHook(() => useWorktreeList("/test/repo"));
			await act(async () => {});

			let resolvePr!: (status: PrStatus) => void;
			const branch = makeBranch({ is_deleting: true });
			setupMockInvoke(
				[branch],
				new Promise<PrStatus>((resolve) => {
					resolvePr = resolve;
				}),
			);
			await act(async () => {
				if (trigger === "refresh") {
					void result.current.refresh({ silent: true });
				} else if (trigger === "branch-list-sync") {
					const subscription = mockListen.mock.calls.find(
						([event]) => event === trigger,
					);
					expect(subscription).toBeDefined();
					subscription?.[1]();
				} else {
					window.dispatchEvent(new Event(trigger));
				}
			});
			expect(result.current.branches).toEqual([branch]);
			expect(result.current.loading).toBe(false);

			await act(async () => {
				resolveOldPr({ open_prs: {}, merged_branches: [] });
			});
			expect(result.current.branches).toEqual([branch]);

			await act(async () => {
				resolvePr({ open_prs: {}, merged_branches: [] });
			});
			expect(result.current.branches).toEqual([branch]);
		},
	);

	it.each(["refresh", "branch-list-sync", "branch-list-refresh", "polling"])(
		"%s中は既存行のPR表示を保持し、削除中状態と最新PR応答を反映する",
		async (trigger) => {
			vi.useFakeTimers();
			try {
				const deleting = makeBranch();
				const other = makeBranch({
					name: "feat/other",
					worktree_path: "/tmp/other",
				});
				const pr = { number: 1845, url: "https://example.com/pr/1845" };
				setupMockInvoke(
					[deleting, other],
					Promise.resolve({
						open_prs: { [deleting.name]: pr, [other.name]: pr },
						merged_branches: [],
					}),
				);
				const { result, unmount } = renderHook(() =>
					useWorktreeList("/test/repo"),
				);
				await act(async () => {});
				expect(result.current.branches.every((branch) => branch.has_pr)).toBe(
					true,
				);

				let resolvePr!: (status: PrStatus) => void;
				const updated = [
					{ ...deleting, is_deleting: true },
					{ ...other, dirty_count: 3 },
				];
				setupMockInvoke(
					updated,
					new Promise((resolve) => {
						resolvePr = resolve;
					}),
				);
				await act(async () => {
					if (trigger === "refresh")
						void result.current.refresh({ silent: true });
					else if (trigger === "polling")
						await vi.advanceTimersByTimeAsync(120_000);
					else if (trigger === "branch-list-sync") {
						mockListen.mock.calls.find(([event]) => event === trigger)?.[1]();
					} else window.dispatchEvent(new Event(trigger));
				});
				expect(result.current.branches).toEqual(
					updated.map((branch) => ({
						...branch,
						has_pr: true,
						pr_number: pr.number,
						pr_url: pr.url,
					})),
				);

				await act(async () => {
					resolvePr({
						open_prs: {
							[other.name]: {
								number: 1900,
								url: "https://example.com/pr/1900",
							},
						},
						merged_branches: [],
					});
				});
				expect(result.current.branches).toEqual([
					updated[0],
					{
						...updated[1],
						has_pr: true,
						pr_number: 1900,
						pr_url: "https://example.com/pr/1900",
					},
				]);
				unmount();
			} finally {
				vi.useRealTimers();
			}
		},
	);

	it.each(["repository", "branch", "worktree"])(
		"%sが変わった行には以前のPR表示を引き継がない",
		async (changed) => {
			const branch = makeBranch();
			setupMockInvoke(
				[branch],
				Promise.resolve({
					open_prs: {
						[branch.name]: { number: 1845, url: "https://example.com/pr/1845" },
					},
					merged_branches: [],
				}),
			);
			const { result, rerender } = renderHook(
				({ repoPath }) => useWorktreeList(repoPath),
				{
					initialProps: { repoPath: "/test/repo" },
				},
			);
			await waitFor(() =>
				expect(result.current.branches[0]?.has_pr).toBe(true),
			);
			const updated = {
				...branch,
				...(changed === "branch" ? { name: "feat/new" } : {}),
				...(changed === "worktree" ? { worktree_path: "/tmp/new" } : {}),
			};
			let resolvePr!: (status: PrStatus) => void;
			setupMockInvoke(
				[updated],
				new Promise((resolve) => {
					resolvePr = resolve;
				}),
			);
			await act(async () => {
				if (changed === "repository") rerender({ repoPath: "/test/other" });
				else void result.current.refresh({ silent: true });
			});
			expect(result.current.branches).toEqual([updated]);
			await act(async () => {
				resolvePr({ open_prs: {}, merged_branches: [] });
			});
		},
	);

	it("遅れて届いた古いsnapshotで削除中の一覧を上書きしない", async () => {
		const { result } = renderHook(() => useWorktreeList("/test/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));

		let resolveSnapshot!: (snapshot: {
			worktree_display_groups: ReturnType<typeof displayGroups>;
		}) => void;
		mockInvoke.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveSnapshot = resolve;
				}),
		);
		let oldRefresh!: Promise<void>;
		await act(async () => {
			oldRefresh = result.current.refresh({ silent: true });
		});

		const branch = makeBranch({ is_deleting: true });
		setupMockInvoke([branch]);
		await act(async () => {
			await result.current.refresh({ silent: true });
		});
		expect(result.current.branches).toEqual([branch]);

		await act(async () => {
			resolveSnapshot({
				worktree_display_groups: displayGroups([makeBranch()]),
			});
			await oldRefresh;
		});
		expect(result.current.branches).toEqual([branch]);
	});

	it("should set loading to true when refresh is called without silent", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data
		const updatedBranch = makeBranch({ dirty_count: 3 });
		setupMockInvoke([updatedBranch]);

		await act(async () => {
			await result.current.refresh();
		});

		// After completion, loading is false
		expect(result.current.loading).toBe(false);
		expect(result.current.branches[0].dirty_count).toBe(3);
	});

	it("should skip setBranches when data has not changed", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(1);
		});

		const firstBranches = result.current.branches;

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		// Same reference because data didn't change
		expect(result.current.branches).toBe(firstBranches);
	});

	it("should update branches when data changes", async () => {
		const branch = makeBranch();
		setupMockInvoke([branch]);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(1);
		});

		const firstBranches = result.current.branches;

		// Return different data
		const newBranch = makeBranch({
			name: "feat/new",
			worktree_path: "/tmp/wt2",
		});
		setupMockInvoke([branch, newBranch]);

		await act(async () => {
			await result.current.refresh({ silent: true });
		});

		expect(result.current.branches).not.toBe(firstBranches);
		expect(result.current.branches).toHaveLength(2);
	});

	it("should filter out branches without worktree_path but include main worktree branches", async () => {
		const branches = [
			makeBranch({ name: "main", is_main_worktree: true }),
			makeBranch({
				name: "feat/a",
				worktree_path: null as unknown as string,
			}),
			makeBranch({ name: "feat/b", worktree_path: "/tmp/b" }),
		];
		setupMockInvoke(branches);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(2);
		});

		const names = result.current.branches.map((b) => b.name);
		expect(names).toContain("main");
		expect(names).toContain("feat/b");
		expect(names).not.toContain("feat/a");
	});

	it("backendのworktree一覧を受け取った順で表示する", async () => {
		setupMockInvoke([
			makeBranch({ name: "second" }),
			makeBranch({ name: "first" }),
		]);
		const { result } = renderHook(() => useWorktreeList("/test/repo"));
		await waitFor(() => {
			expect(result.current.branches.map((branch) => branch.name)).toEqual([
				"second",
				"first",
			]);
		});
	});

	it("should pass is_main_worktree through to branches when main repo is on feature branch", async () => {
		const branches = [
			makeBranch({
				name: "main",
				is_main_worktree: false,
				is_deleting: false,
				worktree_path: null as unknown as string,
			}),
			makeBranch({
				name: "feat/current",
				is_main_worktree: true,
				is_deleting: false,
				worktree_path: "/repo",
			}),
			makeBranch({
				name: "feat/wt",
				is_main_worktree: false,
				is_deleting: false,
				worktree_path: "/tmp/wt",
			}),
		];
		setupMockInvoke(branches);

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.branches).toHaveLength(2);
		});

		const mainWt = result.current.branches.find((b) => b.is_main_worktree);
		expect(mainWt).toBeDefined();
		expect(mainWt?.name).toBe("feat/current");
		expect(mainWt?.is_main_worktree).toBe(true);
	});

	it("should call refresh with silent: true from branch-list-sync event", async () => {
		setupMockInvoke([makeBranch()]);

		type ListenCallback = () => void;
		let branchListSyncCallback: ListenCallback | null = null;
		mockListen.mockImplementation((event: string, cb: ListenCallback) => {
			if (event === "branch-list-sync") {
				branchListSyncCallback = cb;
			}
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeList("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		// Change data so we can verify the event triggers refresh
		const updatedBranch = makeBranch({ dirty_count: 7 });
		setupMockInvoke([updatedBranch]);

		expect(branchListSyncCallback).not.toBeNull();

		await act(async () => {
			branchListSyncCallback?.();
		});

		await waitFor(() => {
			expect(result.current.branches[0].dirty_count).toBe(7);
		});

		// loading should remain false (silent refresh)
		expect(result.current.loading).toBe(false);
	});
	it("PR取得中も削除中は短い間隔で一覧を読み直し、終了後に古い行を復活させない", async () => {
		vi.useFakeTimers();
		try {
			let resolvePr!: (status: PrStatus) => void;
			const prStatus = new Promise<PrStatus>((resolve) => {
				resolvePr = resolve;
			});
			const branch = makeBranch({ is_deleting: true });
			setupMockInvoke([branch], prStatus);
			const { result, unmount } = renderHook(() =>
				useWorktreeList("/test/repo"),
			);
			await act(async () => {});
			expect(result.current.branches[0].is_deleting).toBe(true);
			const count = () =>
				mockInvoke.mock.calls.filter(
					([cmd]) => cmd === "list_branches_with_status_snapshot",
				).length;
			const initial = count();
			await act(async () => {
				await vi.advanceTimersByTimeAsync(1_000);
			});
			expect(count()).toBe(initial + 1);
			expect(result.current.branches).toEqual([branch]);
			let resolveLatestPr!: (status: PrStatus) => void;
			setupMockInvoke(
				[],
				new Promise<PrStatus>((resolve) => {
					resolveLatestPr = resolve;
				}),
			);
			await act(async () => {
				await vi.advanceTimersByTimeAsync(1_000);
			});
			expect(count()).toBe(initial + 2);
			expect(result.current.branches).toEqual([]);
			await act(async () => {
				resolveLatestPr({ open_prs: {}, merged_branches: [] });
			});
			await act(async () => {
				resolvePr({ open_prs: {}, merged_branches: [] });
			});
			expect(result.current.branches).toEqual([]);
			await act(async () => {
				await vi.advanceTimersByTimeAsync(1_000);
			});
			expect(count()).toBe(initial + 2);
			unmount();
		} finally {
			vi.useRealTimers();
		}
	});
});
