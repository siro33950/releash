import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";

export interface FileChangeEvent {
	watcher_id: number;
	path: string;
	kind: string;
}

export interface UseFileWatcherOptions {
	rootPath: string | null;
	onFileChange?: (event: FileChangeEvent) => void;
}

export interface UseFileWatcherReturn {
	watcherId: number | null;
	isWatching: boolean;
	error: string | null;
}

export function useFileWatcher(
	options: UseFileWatcherOptions,
): UseFileWatcherReturn {
	const { rootPath, onFileChange } = options;

	const [watcherId, setWatcherId] = useState<number | null>(null);
	const [error, setError] = useState<string | null>(null);
	const watcherIdRef = useRef<number | null>(null);
	const onFileChangeRef = useRef(onFileChange);
	onFileChangeRef.current = onFileChange;

	const isWatching = watcherId !== null;

	useEffect(() => {
		if (!rootPath) {
			setWatcherId(null);
			setError(null);
			return;
		}

		let isMounted = true;
		let unlisten: UnlistenFn | null = null;

		const startWatching = async () => {
			try {
				unlisten = await listen<FileChangeEvent>("file-change", (event) => {
					if (event.payload.watcher_id === watcherIdRef.current) {
						onFileChangeRef.current?.(event.payload);
					}
				});

				if (!isMounted) {
					unlisten();
					return;
				}

				const id = await invoke<number>("start_watching", { path: rootPath });

				if (!isMounted) {
					invoke("stop_watching", { watcherId: id }).catch(() => {});
					unlisten?.();
					return;
				}

				watcherIdRef.current = id;
				setWatcherId(id);
				setError(null);
			} catch (e) {
				if (isMounted) {
					setError(e instanceof Error ? e.message : String(e));
					setWatcherId(null);
				}
			}
		};

		startWatching();

		return () => {
			isMounted = false;
			unlisten?.();
			if (watcherIdRef.current !== null) {
				invoke("stop_watching", { watcherId: watcherIdRef.current }).catch(
					() => {},
				);
				watcherIdRef.current = null;
			}
		};
	}, [rootPath]);

	return {
		watcherId,
		isWatching,
		error,
	};
}
