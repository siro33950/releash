import { createContext, useContext, useMemo } from "react";
import { useGitStatus } from "@/hooks/useGitStatus";
import type { FileStatus } from "@/types/file-tree";
import type { GitFileStatus } from "@/types/git";

export interface GitStatusContextValue {
	statusMap: Map<string, FileStatus>;
	stagedFiles: GitFileStatus[];
	changedFiles: GitFileStatus[];
	refresh: () => void;
}

const GitStatusContext = createContext<GitStatusContextValue | null>(null);

export function useGitStatusContext(): GitStatusContextValue {
	const ctx = useContext(GitStatusContext);
	if (!ctx) {
		throw new Error(
			"useGitStatusContext must be used within GitStatusProvider",
		);
	}
	return ctx;
}

export function GitStatusProvider({
	rootPath,
	externalRefreshKey,
	children,
}: {
	rootPath: string;
	externalRefreshKey?: number;
	children: React.ReactNode;
}) {
	const { statusMap, stagedFiles, changedFiles, refresh } = useGitStatus(
		rootPath,
		externalRefreshKey,
	);

	const value = useMemo<GitStatusContextValue>(
		() => ({ statusMap, stagedFiles, changedFiles, refresh }),
		[statusMap, stagedFiles, changedFiles, refresh],
	);

	return (
		<GitStatusContext.Provider value={value}>
			{children}
		</GitStatusContext.Provider>
	);
}
