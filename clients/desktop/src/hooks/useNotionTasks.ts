import { useCallback, useEffect, useRef, useState } from "react";
import type { StateValues } from "@/lib/client";
import { useStateSubscriptionResult } from "./useStateSubscription";

const DEBOUNCE_MS = 300;

export function useNotionTasks(repoPath: string) {
	const [request, setRequest] = useState({
		repoPath,
		count: 20,
		filters: [] as string[],
	});
	const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);
	if (request.repoPath !== repoPath) {
		clearTimeout(debounceRef.current);
		setRequest({ repoPath, count: 20, filters: [] });
	}
	const args = [repoPath, String(request.count), ...request.filters];
	const subscription = useStateSubscriptionResult({
		kind: "notion-tasks",
		args,
	});
	const filterKey = JSON.stringify([repoPath, request.filters]);
	const previous = useRef<
		{ key: string; page: StateValues["notion-tasks"]["page"] } | undefined
	>(undefined);
	const page = subscription.value?.page;
	useEffect(() => {
		if (page) previous.current = { key: filterKey, page };
	}, [page, filterKey]);
	const displayed =
		page ??
		(previous.current?.key === filterKey ? previous.current.page : undefined);
	const error =
		subscription.error ?? subscription.value?.readError?.message ?? null;
	const loading = subscription.value === undefined && !error;
	useEffect(() => () => clearTimeout(debounceRef.current), []);
	const search = useCallback(
		(title: string, labels: Record<string, string[]>) => {
			clearTimeout(debounceRef.current);
			debounceRef.current = setTimeout(() => {
				const filters: string[] = [];
				if (title) filters.push(`title=${title}`);
				const entries = Object.entries(labels).filter(
					([, values]) => values.length > 0,
				);
				if (entries.length)
					filters.push(`labels=${JSON.stringify(Object.fromEntries(entries))}`);
				setRequest({ repoPath, count: 20, filters });
			}, DEBOUNCE_MS);
		},
		[repoPath],
	);
	const loadMore = useCallback(() => {
		if (displayed?.has_more && !loading)
			setRequest((current) => ({ ...current, count: current.count + 20 }));
	}, [displayed?.has_more, loading]);
	return {
		tasks: displayed?.tasks ?? [],
		hasMore: displayed?.has_more ?? false,
		loading,
		error,
		readError: subscription.value?.readError,
		search,
		loadMore,
	};
}
