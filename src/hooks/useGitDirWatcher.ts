import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";

export function useGitDirWatcher(repoPath: string | null): void {
	const watcherIdRef = useRef<number | null>(null);

	useEffect(() => {
		if (!repoPath) return;

		let isMounted = true;

		const start = async () => {
			try {
				const id = await invoke<number>("start_git_dir_watching", {
					repoPath,
				});
				if (!isMounted) {
					invoke("stop_watching", { watcherId: id }).catch(() => {});
					return;
				}
				watcherIdRef.current = id;
			} catch (e) {
				console.error("Failed to start git dir watching:", e);
			}
		};

		start();

		return () => {
			isMounted = false;
			if (watcherIdRef.current !== null) {
				invoke("stop_watching", { watcherId: watcherIdRef.current }).catch(
					() => {},
				);
				watcherIdRef.current = null;
			}
		};
	}, [repoPath]);
}
