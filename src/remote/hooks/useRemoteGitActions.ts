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
	const [committing, setCommitting] = useState(false);
	const [pushing, setPushing] = useState(false);
	const [pushResult, setPushResult] = useState<string | null>(null);

	useEffect(() => {
		return subscribe((msg: WsMessage) => {
			if (msg.type === "git_stage_result" && !msg.payload.success) {
				setError(msg.payload.error ?? "Unknown error");
			}
			if (msg.type === "git_commit_result") {
				setCommitting(false);
				if (!msg.payload.success) {
					setError(msg.payload.error ?? "Commit failed");
				}
			}
			if (msg.type === "git_push_result") {
				setPushing(false);
				if (msg.payload.success) {
					setPushResult(msg.payload.output ?? "Push successful");
				} else {
					setError(msg.payload.error ?? "Push failed");
				}
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

	const stageHunk = useCallback(
		(patch: string) => {
			setError(null);
			send({ type: "git_stage_hunk", payload: { patch } });
		},
		[send],
	);

	const commit = useCallback(
		(message: string) => {
			setError(null);
			setCommitting(true);
			send({
				type: "git_commit_request",
				payload: { message },
			});
		},
		[send],
	);

	const push = useCallback(() => {
		setError(null);
		setPushResult(null);
		setPushing(true);
		send({
			type: "git_push_request",
			payload: {} as Record<string, never>,
		});
	}, [send]);

	const clearError = useCallback(() => setError(null), []);
	const clearPushResult = useCallback(() => setPushResult(null), []);

	return {
		stage,
		unstage,
		stageHunk,
		commit,
		push,
		committing,
		pushing,
		pushResult,
		clearPushResult,
		error,
		clearError,
	};
}
