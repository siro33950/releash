import { createContext, useContext } from "react";
import type { UseLspReturn } from "@/hooks/useLsp";
import type { LineComment } from "@/types/comment";
import type { TabInfo } from "@/types/editor";
import type { DiffBase, DiffMode, Theme } from "@/types/settings";

export interface EditorContextValue {
	getFileContent: (path: string) => TabInfo | undefined;
	updateContent: (path: string, content: string) => void;
	saveFile: (path: string) => Promise<void>;

	diffBase: DiffBase;
	diffMode: DiffMode;
	setDiffBase: (base: DiffBase) => void;
	setDiffMode: (mode: DiffMode) => void;

	comments: LineComment[];
	addComment: (
		filePath: string,
		lineNumber: number,
		content: string,
		endLine?: number,
	) => void;
	deleteComment: (id: string) => void;
	resolveComment?: (id: string) => void;
	updateComment: (id: string, content: string) => void;
	sendComment?: (comment: LineComment) => void;
	copyComment?: (comment: LineComment) => void;

	showResolvedComments: boolean;
	toggleShowResolvedComments: () => void;

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
