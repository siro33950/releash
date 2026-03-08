import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef } from "react";
import {
	type WorkspaceState,
	worktreeNameFromPath,
} from "@/types/workspace-state";

export interface UseWorkspaceStateCacheReturn {
	getState: (rootPath: string) => WorkspaceState | undefined;
	loadState: (rootPath: string) => Promise<WorkspaceState | undefined>;
	updateState: (rootPath: string, state: WorkspaceState) => void;
	flushState: (rootPath: string) => void;
}

export function useWorkspaceStateCache(): UseWorkspaceStateCacheReturn {
	const cacheRef = useRef<Map<string, WorkspaceState>>(new Map());
	const timersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
		new Map(),
	);
	const dirtyRef = useRef<Set<string>>(new Set());

	const getState = useCallback((rootPath: string) => {
		return cacheRef.current.get(rootPath);
	}, []);

	const saveToBackend = useCallback((rootPath: string) => {
		const state = cacheRef.current.get(rootPath);
		if (!state) return;
		invoke("save_workspace_state", {
			worktreeName: worktreeNameFromPath(rootPath),
			state,
		}).catch((e) => {
			console.error("Failed to save workspace state:", e);
		});
		dirtyRef.current.delete(rootPath);
	}, []);

	const loadState = useCallback(
		async (rootPath: string): Promise<WorkspaceState | undefined> => {
			try {
				const state = await invoke<WorkspaceState | null>(
					"load_workspace_state",
					{
						worktreeName: worktreeNameFromPath(rootPath),
						worktreeRoot: rootPath,
					},
				);
				if (state) {
					cacheRef.current.set(rootPath, state);
					return state;
				}
				return undefined;
			} catch (e) {
				console.error("Failed to load workspace state:", e);
				return undefined;
			}
		},
		[],
	);

	const updateState = useCallback(
		(rootPath: string, state: WorkspaceState) => {
			cacheRef.current.set(rootPath, state);
			dirtyRef.current.add(rootPath);

			// Debounce save: cancel existing timer and set a new one
			const existingTimer = timersRef.current.get(rootPath);
			if (existingTimer) {
				clearTimeout(existingTimer);
			}
			const timer = setTimeout(() => {
				saveToBackend(rootPath);
				timersRef.current.delete(rootPath);
			}, 500);
			timersRef.current.set(rootPath, timer);
		},
		[saveToBackend],
	);

	const flushState = useCallback(
		(rootPath: string) => {
			// Cancel pending debounce timer
			const existingTimer = timersRef.current.get(rootPath);
			if (existingTimer) {
				clearTimeout(existingTimer);
				timersRef.current.delete(rootPath);
			}
			// Save immediately if dirty
			if (dirtyRef.current.has(rootPath)) {
				saveToBackend(rootPath);
			}
		},
		[saveToBackend],
	);

	// Flush all dirty entries on unmount
	useEffect(() => {
		return () => {
			for (const timer of timersRef.current.values()) {
				clearTimeout(timer);
			}
			for (const rootPath of dirtyRef.current) {
				const state = cacheRef.current.get(rootPath);
				if (!state) continue;
				invoke("save_workspace_state", {
					worktreeName: worktreeNameFromPath(rootPath),
					state,
				}).catch(() => {});
			}
		};
	}, []);

	return { getState, loadState, updateState, flushState };
}
