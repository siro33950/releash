import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { getErrorMessage } from "@/lib/errorMessage";

type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "error";

interface UpdateInfo {
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

	useEffect(() => {
		if (!enabled) return;

		let cancelled = false;

		(async () => {
			setStatus("checking");
			try {
				const update = await invoke<UpdateInfo | null>("check_desktop_update");
				if (cancelled) return;

				if (update) {
					setUpdateInfo({
						version: update.version,
						notes: update.notes,
					});
					setStatus("available");
				} else {
					setStatus("idle");
				}
			} catch {
				if (!cancelled) setStatus("idle");
			}
		})();

		return () => {
			cancelled = true;
		};
	}, [enabled]);

	const downloadAndInstall = useCallback(() => {
		if (!updateInfo) return;

		(async () => {
			setStatus("downloading");
			setProgress(0);
			let unlisten: UnlistenFn | undefined;
			try {
				unlisten = await listen<number>("desktop-update-progress", (event) =>
					setProgress(event.payload),
				);
				await invoke("install_desktop_update");
			} catch (e) {
				setError(getErrorMessage(e));
				setStatus("error");
			} finally {
				unlisten?.();
			}
		})();
	}, [updateInfo]);

	const dismiss = useCallback(() => {
		setStatus("idle");
		setUpdateInfo(null);
		setError(null);
	}, []);

	return { status, updateInfo, progress, error, downloadAndInstall, dismiss };
}
