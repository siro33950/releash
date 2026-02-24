import { useCallback, useEffect, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface PtySession {
	ptyId: number;
	cols: number;
	label?: string;
	worktreePath?: string;
}

interface UsePtyManagementOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
}

export function usePtyManagement({ subscribe, send }: UsePtyManagementOptions) {
	const [ptySessions, setPtySessions] = useState<PtySession[]>([]);
	const [activePtyId, setActivePtyId] = useState<number | null>(null);
	const [ptySpawning, setPtySpawning] = useState(false);
	const [ptySpawnError, setPtySpawnError] = useState<string | null>(null);
	const [terminalMounted, setTerminalMounted] = useState(false);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "pty_spawn_response") {
				setPtySpawning(false);
				if (!msg.payload.success) {
					setPtySpawnError(msg.payload.error ?? "PTY起動に失敗しました");
				}
			}
			if (msg.type === "pty_ready") {
				const { pty_id, cols, label, worktree_path } = msg.payload;
				setPtySessions((prev) => {
					if (prev.some((s) => s.ptyId === pty_id)) return prev;
					return [
						...prev,
						{
							ptyId: pty_id,
							cols,
							label,
							worktreePath: worktree_path,
						},
					];
				});
				setActivePtyId((prev) => prev ?? pty_id);
				setPtySpawnError(null);
			}
			if (msg.type === "pty_exit") {
				const { pty_id } = msg.payload;
				setPtySessions((prev) => prev.filter((s) => s.ptyId !== pty_id));
				setActivePtyId((prev) => (prev === pty_id ? null : prev));
			}
			if (msg.type === "worktree_select_response" && msg.payload.success) {
				// セッション保持: activePtyIdのみリセット（直後にPtyReadyが流れるので復元される）
				setActivePtyId(null);
			}
		});
	}, [subscribe]);

	const spawnPty = useCallback(
		(label?: string) => {
			setPtySpawnError(null);
			setPtySpawning(true);
			send({
				type: "pty_spawn_request",
				payload: { cols: 80, rows: 24, label },
			});
		},
		[send],
	);

	const killPty = useCallback(
		(ptyId: number) => {
			send({
				type: "pty_kill_request",
				payload: { pty_id: ptyId },
			});
		},
		[send],
	);

	const resetPty = useCallback(() => {
		setPtySessions([]);
		setActivePtyId(null);
		setPtySpawnError(null);
		setPtySpawning(false);
	}, []);

	return {
		ptySessions,
		activePtyId,
		ptySpawning,
		ptySpawnError,
		terminalMounted,
		setActivePtyId,
		setTerminalMounted,
		spawnPty,
		killPty,
		resetPty,
	};
}
