import { createContext, useContext } from "react";
import type { UseLspReturn } from "@/hooks/useLsp";
import type { TabInfo } from "@/types/editor";
import type { DiffBase, DiffMode, Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";

export interface EditorContextValue {
	getFileContent: (path: string) => TabInfo | undefined;
	updateContent: (path: string, content: string) => void;
	saveFile: (path: string) => Promise<void>;

	diffBase: DiffBase;
	diffMode: DiffMode;
	setDiffBase: (base: DiffBase) => void;
	setDiffMode: (mode: DiffMode) => void;

	threads: Thread[];
	createThread: (
		filePath: string,
		lineNumber: number,
		content: string,
		endLine?: number,
		fileContent?: string,
	) => void;
	addEntry: (threadId: string, content: string) => void;
	deleteThread: (threadId: string) => void;
	resolveThread?: (threadId: string) => void;
	implementThread?: (threadId: string) => void;
	updateEntry: (threadId: string, entryId: string, content: string) => void;
	onPostToPr?: (threadId: string) => void;
	aiRunningThreadIds?: Set<string>;
	aiTaskThreadIds?: Set<string>;
	onOpenThreadAIModal?: (threadId?: string) => void;
	onAskAI?: (threadId: string) => void;
	sendThread?: (thread: Thread) => void;
	copyThread?: (thread: Thread) => void;

	recalculateAnchorsForFile?: (
		filePath: string,
		currentContent: string,
	) => void;
	showResolvedThreads: boolean;
	toggleShowResolvedThreads: () => void;

	rootPath: string;
	onStageHunk?: (repoPath: string, patch: string) => Promise<void>;
	onGitChanged?: () => void;
	gitRefreshKey: number;

	theme?: Theme;
	fontSize?: number;
	onSearchOccurrences?: (text: string) => void;

	lspStatus: UseLspReturn["status"];
	lspError: string | null;
	lspCrashCount: number;
	lspRetryManually: () => void;
}

export const EditorContext = createContext<EditorContextValue | null>(null);

export function useEditorContext(): EditorContextValue {
	const ctx = useContext(EditorContext);
	if (!ctx) {
		throw new Error(
			"useEditorContext must be used within EditorContext.Provider",
		);
	}
	return ctx;
}
