import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";
import type { FileChangeEvent } from "./useFileWatcher";

export function useGitEventRefresh(
	rootPath: string | null,
	onRefresh: () => void,
	enabled = true,
): void {
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const watcherIdRef = useRef<number | null>(null);

	const debouncedRefresh = useCallback(() => {
		if (timerRef.current) clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			onRefresh();
		}, 300);
	}, [onRefresh]);

	useEffect(() => {
		if (!enabled || !rootPath) return;

		let unlisten: UnlistenFn | null = null;
		let disposed = false;

		const setup = async () => {
			const off = await listen<FileChangeEvent>("file-change", (event) => {
				if (
					!disposed &&
					watcherIdRef.current !== null &&
					event.payload.watcher_id === watcherIdRef.current
				) {
					debouncedRefresh();
				}
			});
			if (disposed) {
				off();
				return;
			}
			unlisten = off;

			try {
				const id = await invoke<number>("start_watching", {
					path: rootPath,
				});
				if (disposed) {
					invoke("stop_watching", { watcherId: id }).catch(() => {});
					return;
				}
				watcherIdRef.current = id;
			} catch (e) {
				console.error("Failed to start file watcher:", e);
			}
		};
		void setup();

		return () => {
			disposed = true;
			unlisten?.();
			if (timerRef.current) clearTimeout(timerRef.current);
			if (watcherIdRef.current !== null) {
				invoke("stop_watching", { watcherId: watcherIdRef.current }).catch(
					() => {},
				);
				watcherIdRef.current = null;
			}
		};
	}, [debouncedRefresh, rootPath, enabled]);

	useEffect(() => {
		if (!enabled || !rootPath) return;

		let unlisten: UnlistenFn | null = null;
		let disposed = false;

		const setup = async () => {
			const off = await listen<{ repo_path: string }>(
				"git-status-changed",
				(event) => {
					if (!disposed && event.payload.repo_path === rootPath) {
						debouncedRefresh();
					}
				},
			);
			if (disposed) {
				off();
				return;
			}
			unlisten = off;
		};
		setup();

		return () => {
			disposed = true;
			unlisten?.();
		};
	}, [debouncedRefresh, rootPath, enabled]);
}
