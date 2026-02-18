import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { NotionTaskFilters } from "./useNotionTasks";
import { DEBOUNCE_MS, useNotionTasks } from "./useNotionTasks";

describe("useNotionTasks", () => {
	it("should invoke query_notion_tasks on mount", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue({
			tasks: [],
			has_more: false,
			next_cursor: null,
		});

		const { result } = renderHook(() => useNotionTasks("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(invoke).toHaveBeenCalledWith("query_notion_tasks", {
			repoPath: "/test/repo",
			query: {
				title_filter: "",
				label_filters: {},
				cursor: null,
			},
		});
	});

	it("should set tasks from query result", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const mockPage = {
			tasks: [
				{
					id: "page-1",
					title: "Test Task",
					url: "https://notion.so/page-1",
					labels: { Status: ["Todo"] },
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: true,
			next_cursor: "cursor-abc",
		};
		vi.mocked(invoke).mockResolvedValue(mockPage);

		const { result } = renderHook(() => useNotionTasks("/test/repo"));

		await waitFor(() => {
			expect(result.current.tasks).toHaveLength(1);
		});

		expect(result.current.tasks[0].title).toBe("Test Task");
		expect(result.current.hasMore).toBe(true);
	});

	it("should append tasks on loadMore", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		const page1 = {
			tasks: [
				{
					id: "page-1",
					title: "Task 1",
					url: "https://notion.so/page-1",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: true,
			next_cursor: "cursor-abc",
		};
		const page2 = {
			tasks: [
				{
					id: "page-2",
					title: "Task 2",
					url: "https://notion.so/page-2",
					labels: {},
					branch_name: "",
					created_at: "2026-01-01T00:00:00.000Z",
					last_edited_at: "2026-01-01T00:00:00.000Z",
				},
			],
			has_more: false,
			next_cursor: null,
		};

		vi.mocked(invoke).mockResolvedValueOnce(page1).mockResolvedValueOnce(page2);

		const { result } = renderHook(() => useNotionTasks("/test/repo"));

		await waitFor(() => {
			expect(result.current.tasks).toHaveLength(1);
		});

		await act(async () => {
			result.current.loadMore();
		});

		await waitFor(() => {
			expect(result.current.tasks).toHaveLength(2);
		});

		expect(result.current.tasks[0].title).toBe("Task 1");
		expect(result.current.tasks[1].title).toBe("Task 2");
		expect(result.current.hasMore).toBe(false);
	});

	it("should set empty tasks on error", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockRejectedValue(new Error("not configured"));

		const { result } = renderHook(() => useNotionTasks("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(result.current.tasks).toEqual([]);
	});

	it("should reset and refetch on refresh", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue({
			tasks: [],
			has_more: false,
			next_cursor: null,
		});

		const { result } = renderHook(() => useNotionTasks("/test/repo"));

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		vi.mocked(invoke).mockClear();
		vi.mocked(invoke).mockResolvedValue({
			tasks: [],
			has_more: false,
			next_cursor: null,
		});

		await act(async () => {
			result.current.refresh();
		});

		expect(invoke).toHaveBeenCalledWith("query_notion_tasks", {
			repoPath: "/test/repo",
			query: {
				title_filter: "",
				label_filters: {},
				cursor: null,
			},
		});
	});

	it("should use initialFilters on mount", async () => {
		const { invoke } = await import("@tauri-apps/api/core");
		vi.mocked(invoke).mockResolvedValue({
			tasks: [],
			has_more: false,
			next_cursor: null,
		});

		const initialFilters: NotionTaskFilters = {
			title: "saved query",
			labels: { Status: "In Progress" },
		};

		const { result } = renderHook(() =>
			useNotionTasks("/test/repo", initialFilters),
		);

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});

		expect(invoke).toHaveBeenCalledWith("query_notion_tasks", {
			repoPath: "/test/repo",
			query: {
				title_filter: "saved query",
				label_filters: { Status: "In Progress" },
				cursor: null,
			},
		});
	});

	it("should debounce search calls", async () => {
		vi.useFakeTimers();
		try {
			const { invoke } = await import("@tauri-apps/api/core");
			vi.mocked(invoke).mockResolvedValue({
				tasks: [],
				has_more: false,
				next_cursor: null,
			});

			const { result } = renderHook(() => useNotionTasks("/test/repo"));

			await act(async () => {
				await vi.runAllTimersAsync();
			});

			vi.mocked(invoke).mockClear();
			vi.mocked(invoke).mockResolvedValue({
				tasks: [],
				has_more: false,
				next_cursor: null,
			});

			act(() => {
				result.current.search("query", { Status: "Todo" });
			});

			expect(invoke).not.toHaveBeenCalled();

			await act(async () => {
				await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
			});

			expect(invoke).toHaveBeenCalledWith("query_notion_tasks", {
				repoPath: "/test/repo",
				query: {
					title_filter: "query",
					label_filters: { Status: "Todo" },
					cursor: null,
				},
			});
		} finally {
			vi.useRealTimers();
		}
	});
});
