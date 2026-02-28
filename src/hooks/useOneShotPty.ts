import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";

export type OneShotStatus =
	| "starting"
	| "running"
	| "completed"
	| "error"
	| "cancelled"
	| "timeout";

export interface OneShotPtyInfo {
	pty_id: number;
	session_key: string;
	worktree_path: string;
	label: string;
	status: OneShotStatus;
	exit_code: number | null;
	started_at: number;
	completed_at: number | null;
}

export function useOneShotPty() {
	const [activePtys, setActivePtys] = useState<Map<number, OneShotPtyInfo>>(
		new Map(),
	);
	const [outputs, setOutputs] = useState<Map<number, string>>(new Map());

	useEffect(() => {
		const unlistenStatus = listen<OneShotPtyInfo>(
			"oneshot-pty-status-changed",
			(event) => {
				const info = event.payload;
				setActivePtys((prev) => {
					const next = new Map(prev);
					next.set(info.pty_id, info);
					return next;
				});
			},
		);

		const unlistenOutput = listen<{ pty_id: number; data: string }>(
			"pty-output",
			(event) => {
				const { pty_id, data } = event.payload;
				setOutputs((prev) => {
					if (!activePtys.has(pty_id)) return prev;
					const next = new Map(prev);
					const existing = next.get(pty_id) ?? "";
					next.set(pty_id, existing + data);
					return next;
				});
			},
		);

		return () => {
			unlistenStatus.then((f) => f());
			unlistenOutput.then((f) => f());
		};
	}, [activePtys]);

	const spawn = useCallback(
		async (
			command: string,
			worktreePath: string,
			label: string,
			timeoutSecs?: number,
		): Promise<OneShotPtyInfo> => {
			const info = await invoke<OneShotPtyInfo>("spawn_oneshot_pty", {
				command,
				worktreePath,
				label,
				timeoutSecs: timeoutSecs ?? null,
			});
			setActivePtys((prev) => {
				const next = new Map(prev);
				next.set(info.pty_id, info);
				return next;
			});
			return info;
		},
		[],
	);

	const cancel = useCallback(async (ptyId: number): Promise<void> => {
		await invoke("cancel_oneshot_pty", { ptyId });
	}, []);

	const getOutput = useCallback(
		(ptyId: number): string => {
			return outputs.get(ptyId) ?? "";
		},
		[outputs],
	);

	return {
		activePtys,
		spawn,
		cancel,
		getOutput,
	};
}
