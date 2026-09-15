import { useEffect } from "react";
import { watchClient } from "@/lib/clientSocket";

export function useGitDirWatcher(repoPath: string | null): void {
	useEffect(() => {
		if (!repoPath) return;
		return watchClient(
			"start_git_dir_watching",
			{ repoPath },
			() => {},
			(error) => console.error("Failed to start git dir watching:", error),
		);
	}, [repoPath]);
}
