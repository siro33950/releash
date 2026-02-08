import { useEffect, useState } from "react";
import type { GitFileStatus } from "@/types/git";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteGitStatusOptions {
	subscribe: Subscribe;
}

export function useRemoteGitStatus({ subscribe }: UseRemoteGitStatusOptions) {
	const [files, setFiles] = useState<GitFileStatus[]>([]);

	useEffect(() => {
		return subscribe((msg: WsMessage) => {
			if (msg.type === "git_status_sync") {
				setFiles(msg.payload.files);
			} else if (msg.type === "git_stage_result" && msg.payload.success) {
				setFiles(msg.payload.files);
			}
		});
	}, [subscribe]);

	const stagedFiles = files.filter((f) => f.index_status !== "none");
	const changedFiles = files.filter(
		(f) => f.worktree_status !== "none" && f.worktree_status !== "ignored",
	);

	return { files, stagedFiles, changedFiles };
}
