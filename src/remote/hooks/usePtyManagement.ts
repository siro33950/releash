import { useCallback, useEffect, useMemo, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

export interface PtySession {
	ptyId: number;
	cols: number;
	label?: string;
	worktreePath?: string;
	kind: "agent" | "terminal" | "one_shot";
}

interface UsePtyManagementOptions {
	subscribe: Subscribe;
	send: (msg: WsMessage) => void;
}

function inferKindFromLabel(label?: string): PtySession["kind"] {
	if (label && /^agent\b/i.test(label)) return "agent";
	return "terminal";
}

export function usePtyManagement({ subscribe, send }: UsePtyManagementOptions) {
	const [ptySessions, setPtySessions] = useState<PtySession[]>([]);
	const [activePtyId, setActivePtyId] = useState<number | null>(null);
	const [activeAgentPtyId, setActiveAgentPtyId] = useState<number | null>(null);
	const [ptySpawning, setPtySpawning] = useState(false);
	const [ptySpawnError, setPtySpawnError] = useState<string | null>(null);
	const [terminalMounted, setTerminalMounted] = useState(false);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "pty_spawn_response") {
				setPtySpawning(false);
				if (!msg.payload.success) {
					setPtySpawnError(msg.payload.error ?? "Failed to start PTY");
				}
			}
			if (msg.type === "pty_ready") {
				const { pty_id, cols, label, worktree_path, kind } = msg.payload;
				const sessionKind = kind ?? inferKindFromLabel(label);
				setPtySessions((prev) => {
					if (prev.some((s) => s.ptyId === pty_id)) return prev;
					return [
						...prev,
						{
							ptyId: pty_id,
							cols,
							label,
							worktreePath: worktree_path,
							kind: sessionKind,
						},
					];
				});
				if (sessionKind === "agent") {
					setActiveAgentPtyId((prev) => prev ?? pty_id);
				} else {
					setActivePtyId((prev) => prev ?? pty_id);
				}
				setPtySpawnError(null);
			}
			if (msg.type === "pty_exit") {
				const { pty_id } = msg.payload;
				setPtySessions((prev) => prev.filter((s) => s.ptyId !== pty_id));
				setActivePtyId((prev) => (prev === pty_id ? null : prev));
				setActiveAgentPtyId((prev) => (prev === pty_id ? null : prev));
			}
			if (msg.type === "worktree_select_response" && msg.payload.success) {
				setPtySessions([]);
				setActivePtyId(null);
				setActiveAgentPtyId(null);
			}
		});
	}, [subscribe]);

	const terminalSessions = useMemo(
		() => ptySessions.filter((s) => s.kind !== "agent"),
		[ptySessions],
	);

	const agentSessions = useMemo(
		() => ptySessions.filter((s) => s.kind === "agent"),
		[ptySessions],
	);

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
		setActiveAgentPtyId(null);
		setPtySpawnError(null);
		setPtySpawning(false);
	}, []);

	return {
		ptySessions,
		terminalSessions,
		agentSessions,
		activePtyId,
		activeAgentPtyId,
		ptySpawning,
		ptySpawnError,
		terminalMounted,
		setActivePtyId,
		setActiveAgentPtyId,
		setTerminalMounted,
		spawnPty,
		killPty,
		resetPty,
	};
}
