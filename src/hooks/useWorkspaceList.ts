import {
	createContext,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { WorkspaceListSnapshotDto } from "@/generated/client_types";
import { subscribeAgentSessionChanged } from "@/lib/agentSessionEvents";
import {
	invokeClient as invoke,
	listenClient as listen,
	watchClient,
} from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

export interface WorkspaceListModel {
	snapshot: WorkspaceListSnapshotDto | null;
	requestError: { message: string; path?: string } | null;
	refresh: () => Promise<WorkspaceListSnapshotDto | null>;
	refreshRepository: (
		repoPath: string,
	) => Promise<WorkspaceListSnapshotDto | null>;
	refreshWorktree: (
		worktreePath: string,
	) => Promise<WorkspaceListSnapshotDto | null>;
}

export const WorkspaceListContext = createContext<WorkspaceListModel | null>(
	null,
);

export function useWorkspaceList(): WorkspaceListModel {
	const [snapshot, setSnapshot] = useState<WorkspaceListSnapshotDto | null>(
		null,
	);
	const [requestError, setRequestError] =
		useState<WorkspaceListModel["requestError"]>(null);
	const requestsRef = useRef(
		Promise.resolve<WorkspaceListSnapshotDto | null>(null),
	);
	const pendingFullRequestRef =
		useRef<Promise<WorkspaceListSnapshotDto | null> | null>(null);
	const pendingReadRequestRef =
		useRef<Promise<WorkspaceListSnapshotDto | null> | null>(null);
	const request = useCallback(
		(worktreePath?: string, repoPath?: string, readOnly = false) => {
			const full = worktreePath === undefined && repoPath === undefined;
			const pending = readOnly
				? pendingReadRequestRef
				: full
					? pendingFullRequestRef
					: null;
			if (pending?.current) return pending.current;
			const next = requestsRef.current.then(async () => {
				if (pending) pending.current = null;
				try {
					const result = readOnly
						? await invoke("get_workspaces", {})
						: await invoke("refresh_workspaces", {
								worktreePath,
								...(repoPath === undefined ? {} : { repoPath }),
							});
					setSnapshot(result);
					if (!readOnly) setRequestError(null);
					return result;
				} catch (error) {
					if (readOnly) {
						console.warn("[useWorkspaceList] get_workspaces failed", error);
						return null;
					}
					setRequestError({
						message: getErrorMessage(error),
						path: worktreePath ?? repoPath,
					});
					return null;
				}
			});
			if (pending) pending.current = next;
			requestsRef.current = next;
			return next;
		},
		[],
	);
	const readSnapshot = useCallback(() => {
		void request(undefined, undefined, true);
	}, [request]);

	const refresh = useCallback(() => request(), [request]);
	const refreshWorktree = useCallback(
		(path: string) => request(path),
		[request],
	);

	const refreshRepository = useCallback(
		(path: string) => request(undefined, path),
		[request],
	);

	useEffect(() => {
		let active = true;
		const unlisten = listen(
			"workspace-list-changed",
			readSnapshot,
			readSnapshot,
		).then((fn) => {
			if (active) void refresh();
			return fn;
		});
		return () => {
			active = false;
			void unlisten.then((fn) => fn());
		};
	}, [readSnapshot, refresh]);

	const pollInterval = snapshot?.repositories.some((repo) =>
		repo.branches.some((branch) => branch.is_deleting),
	)
		? 1_000
		: 120_000;

	useEffect(() => {
		let active = true;
		const reload = () => {
			if (active) void refresh();
		};
		let timer: ReturnType<typeof setTimeout> | undefined;
		const scheduleReload = () => {
			if (!active) return;
			clearTimeout(timer);
			timer = setTimeout(reload, 80);
		};
		const unlisteners = [
			listen("branch-list-sync", reload, reload),
			listen("repo-paths-changed", reload),
			listen("workflow-execution-changed", scheduleReload, scheduleReload),
		];
		const unsubscribeSessions = subscribeAgentSessionChanged(scheduleReload);
		window.addEventListener("branch-list-refresh", reload);
		window.addEventListener("workspace-tree-refresh", scheduleReload);
		const interval = setInterval(() => {
			if (document.visibilityState === "visible") reload();
		}, pollInterval);
		return () => {
			active = false;
			clearTimeout(timer);
			clearInterval(interval);
			unsubscribeSessions();
			window.removeEventListener("branch-list-refresh", reload);
			window.removeEventListener("workspace-tree-refresh", scheduleReload);
			for (const unlisten of unlisteners) void unlisten.then((fn) => fn());
		};
	}, [refresh, pollInterval]);

	const watchedPaths = JSON.stringify(
		snapshot?.repositories.map((repo) => repo.path) ?? [],
	);
	useEffect(() => {
		const paths: string[] = JSON.parse(watchedPaths);
		const releases = paths.map((repoPath) =>
			watchClient("start_git_dir_watching", { repoPath }, () => {}),
		);
		return () => {
			for (const release of releases) release();
		};
	}, [watchedPaths]);

	return useMemo(
		() => ({
			snapshot,
			requestError,
			refresh,
			refreshWorktree,
			refreshRepository,
		}),
		[snapshot, requestError, refresh, refreshWorktree, refreshRepository],
	);
}
