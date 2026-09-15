import { listenClient as listen } from "@/lib/clientSocket";

type UnlistenFn = () => void;

import { useCallback, useEffect, useRef } from "react";
import { watchClient } from "@/lib/clientSocket";

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
		let stopWatch: (() => void) | undefined;

		const setup = async () => {
			const off = await listen(
				"file-change",
				(event) => {
					if (
						!disposed &&
						watcherIdRef.current !== null &&
						event.payload.watcher_id === watcherIdRef.current
					) {
						debouncedRefresh();
					}
				},
				debouncedRefresh,
			);
			if (disposed) {
				off();
				return;
			}
			unlisten = off;

			stopWatch = watchClient(
				"start_watching",
				{ path: rootPath },
				(id) => {
					watcherIdRef.current = id;
				},
				(error) => console.error("Failed to start file watcher:", error),
			);
		};
		void setup();

		return () => {
			disposed = true;
			unlisten?.();
			if (timerRef.current) clearTimeout(timerRef.current);
			stopWatch?.();
			watcherIdRef.current = null;
		};
	}, [debouncedRefresh, rootPath, enabled]);

	useEffect(() => {
		if (!enabled || !rootPath) return;

		let unlisten: UnlistenFn | null = null;
		let disposed = false;

		const setup = async () => {
			const off = await listen(
				"git-status-changed",
				(event) => {
					if (!disposed && event.payload.repo_path === rootPath) {
						debouncedRefresh();
					}
				},
				debouncedRefresh,
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
