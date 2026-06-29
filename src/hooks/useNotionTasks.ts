import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { NotionTask, NotionTaskPage } from "@/types/notion";

const DEBOUNCE_MS = 300;

export interface NotionTaskFilters {
	title: string;
	labels: Record<string, string[]>;
}

export function useNotionTasks(
	repoPath: string,
	initialFilters?: NotionTaskFilters,
) {
	const [tasks, setTasks] = useState<NotionTask[]>([]);
	const [loading, setLoading] = useState(true);
	const [hasMore, setHasMore] = useState(false);
	const [cursor, setCursor] = useState<string | null>(null);
	const filtersRef = useRef<NotionTaskFilters>(
		initialFilters ?? { title: "", labels: {} },
	);
	const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);

	const fetchTasks = useCallback(
		async (
			title: string,
			labels: Record<string, string[]>,
			startCursor: string | null,
			append: boolean,
		) => {
			setLoading(true);
			try {
				const result = await invoke<NotionTaskPage>("query_notion_tasks", {
					repoPath,
					query: {
						title_filter: title,
						label_filters: labels,
						cursor: startCursor,
					},
				});
				if (append) {
					setTasks((prev) => [...prev, ...result.tasks]);
				} else {
					setTasks(result.tasks);
				}
				setHasMore(result.has_more);
				setCursor(result.next_cursor);
			} catch {
				if (!append) {
					setTasks([]);
				}
				setHasMore(false);
				setCursor(null);
			} finally {
				setLoading(false);
			}
		},
		[repoPath],
	);

	useEffect(() => {
		const { title, labels } = filtersRef.current;
		fetchTasks(title, labels, null, false);
	}, [fetchTasks]);

	useEffect(() => {
		return () => {
			if (debounceRef.current) {
				clearTimeout(debounceRef.current);
			}
		};
	}, []);

	const search = useCallback(
		(title: string, labels: Record<string, string[]>) => {
			filtersRef.current = { title, labels };
			if (debounceRef.current) {
				clearTimeout(debounceRef.current);
			}
			debounceRef.current = setTimeout(() => {
				fetchTasks(title, labels, null, false);
			}, DEBOUNCE_MS);
		},
		[fetchTasks],
	);

	const loadMore = useCallback(() => {
		if (!hasMore || !cursor || loading) return;
		const { title, labels } = filtersRef.current;
		fetchTasks(title, labels, cursor, true);
	}, [hasMore, cursor, loading, fetchTasks]);

	const refresh = useCallback(() => {
		const { title, labels } = filtersRef.current;
		fetchTasks(title, labels, null, false);
	}, [fetchTasks]);

	return {
		tasks,
		loading,
		hasMore,
		search,
		loadMore,
		refresh,
	};
}
