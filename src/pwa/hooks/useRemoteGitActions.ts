import { useCallback, useEffect, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteGitActionsOptions {
	send: (msg: WsMessage) => void;
	subscribe: Subscribe;
}

export function useRemoteGitActions({
	send,
	subscribe,
}: UseRemoteGitActionsOptions) {
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		return subscribe((msg: WsMessage) => {
			if (msg.type === "git_stage_result" && !msg.payload.success) {
				setError(msg.payload.error ?? "Unknown error");
			}
		});
	}, [subscribe]);

	const stage = useCallback(
		(paths: string[]) => {
			setError(null);
			send({ type: "git_stage", payload: { paths } });
		},
		[send],
	);

	const unstage = useCallback(
		(paths: string[]) => {
			setError(null);
			send({ type: "git_unstage", payload: { paths } });
		},
		[send],
	);

	const clearError = useCallback(() => setError(null), []);

	return { stage, unstage, error, clearError };
}
