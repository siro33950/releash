import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef } from "react";
import type { FileChangeEvent } from "./useFileWatcher";

export function useGitEventRefresh(
	rootPath: string | null,
	onRefresh: () => void,
	enabled = true,
): void {
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	const debouncedRefresh = useCallback(() => {
		if (timerRef.current) clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			onRefresh();
		}, 300);
	}, [onRefresh]);

	useEffect(() => {
		if (!enabled || !rootPath) return;

		let unlisten: UnlistenFn | null = null;
		let mounted = true;

		const setup = async () => {
			unlisten = await listen<FileChangeEvent>("file-change", (event) => {
				if (
					mounted &&
					(event.payload.path === rootPath ||
						event.payload.path.startsWith(`${rootPath}/`))
				) {
					debouncedRefresh();
				}
			});
		};
		setup();

		return () => {
			mounted = false;
			unlisten?.();
			if (timerRef.current) clearTimeout(timerRef.current);
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
