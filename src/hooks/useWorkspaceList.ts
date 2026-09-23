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
	error: string | null;
	repositoryErrors: Readonly<Record<string, string>>;
	worktreeErrors: Readonly<Record<string, string>>;
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
	const [error, setError] = useState<string | null>(null);
	const [repositoryErrors, setRepositoryErrors] = useState<
		Record<string, string>
	>({});
	const [worktreeErrors, setWorktreeErrors] = useState<Record<string, string>>(
		{},
	);
	const requestsRef = useRef(
		Promise.resolve<WorkspaceListSnapshotDto | null>(null),
	);
	const request = useCallback((worktreePath?: string, repoPath?: string) => {
		const next = requestsRef.current.then(async () => {
			try {
				const result = await invoke("refresh_workspaces", {
					worktreePath,
					...(repoPath === undefined ? {} : { repoPath }),
				});
				setSnapshot(result);
				if (worktreePath === undefined) {
					setRepositoryErrors((current) => {
						const next = { ...current };
						for (const repo of result.repositories) {
							if (repoPath !== undefined && repo.path !== repoPath) continue;
							if (repo.status.loaded && !repo.status.error)
								delete next[repo.path];
						}
						return Object.keys(current).length === Object.keys(next).length
							? current
							: next;
					});
				}
				setWorktreeErrors((current) => {
					const next = { ...current };
					for (const repo of result.repositories) {
						if (repoPath !== undefined && repo.path !== repoPath) continue;
						for (const tree of repo.worktrees) {
							if (worktreePath !== undefined && tree.path !== worktreePath)
								continue;
							if (tree.status.loaded && !tree.status.error)
								delete next[tree.path];
						}
					}
					return Object.keys(current).length === Object.keys(next).length
						? current
						: next;
				});
				if (
					worktreePath === undefined &&
					repoPath === undefined &&
					result.status.loaded &&
					!result.status.error
				)
					setError(null);
				return result;
			} catch (error) {
				if (worktreePath !== undefined) {
					setWorktreeErrors((current) => ({
						...current,
						[worktreePath]: getErrorMessage(error),
					}));
				} else if (repoPath !== undefined) {
					setRepositoryErrors((current) => ({
						...current,
						[repoPath]: getErrorMessage(error),
					}));
				} else {
					setError(getErrorMessage(error));
				}
				return null;
			}
		});
		requestsRef.current = next;
		return next;
	}, []);
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
		void refresh();
	}, [refresh]);

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
			error,
			repositoryErrors,
			worktreeErrors,
			refresh,
			refreshWorktree,
			refreshRepository,
		}),
		[
			snapshot,
			error,
			repositoryErrors,
			worktreeErrors,
			refresh,
			refreshWorktree,
			refreshRepository,
		],
	);
}
