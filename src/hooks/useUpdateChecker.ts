import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import { useCallback, useEffect, useRef, useState } from "react";

export type UpdateStatus =
	| "idle"
	| "checking"
	| "available"
	| "downloading"
	| "error";

export interface UpdateInfo {
	version: string;
	notes: string;
}

export interface UpdateCheckResult {
	status: UpdateStatus;
	updateInfo: UpdateInfo | null;
	progress: number;
	error: string | null;
	downloadAndInstall: () => void;
	dismiss: () => void;
}

export function useUpdateChecker(enabled: boolean): UpdateCheckResult {
	const [status, setStatus] = useState<UpdateStatus>("idle");
	const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
	const [progress, setProgress] = useState(0);
	const [error, setError] = useState<string | null>(null);
	const checkedRef = useRef(false);
	const updateRef = useRef<Awaited<ReturnType<typeof check>> | null>(null);

	useEffect(() => {
		if (!enabled || checkedRef.current) return;
		checkedRef.current = true;

		let cancelled = false;

		(async () => {
			setStatus("checking");
			try {
				const update = await check();
				if (cancelled) return;

				if (update) {
					updateRef.current = update;
					setUpdateInfo({
						version: update.version,
						notes: update.body ?? "",
					});
					setStatus("available");
				} else {
					setStatus("idle");
				}
			} catch (e) {
				if (cancelled) return;
				setError(e instanceof Error ? e.message : String(e));
				setStatus("error");
			}
		})();

		return () => {
			cancelled = true;
		};
	}, [enabled]);

	const downloadAndInstall = useCallback(() => {
		const update = updateRef.current;
		if (!update) return;

		(async () => {
			setStatus("downloading");
			setProgress(0);
			try {
				let totalLength = 0;
				let downloaded = 0;
				await update.downloadAndInstall((event) => {
					if (event.event === "Started" && event.data.contentLength) {
						totalLength = event.data.contentLength;
					} else if (event.event === "Progress") {
						downloaded += event.data.chunkLength;
						if (totalLength > 0) {
							setProgress(Math.round((downloaded / totalLength) * 100));
						}
					}
				});
				await relaunch();
			} catch (e) {
				setError(e instanceof Error ? e.message : String(e));
				setStatus("error");
			}
		})();
	}, []);

	const dismiss = useCallback(() => {
		setStatus("idle");
		setUpdateInfo(null);
		setError(null);
	}, []);

	return { status, updateInfo, progress, error, downloadAndInstall, dismiss };
}
