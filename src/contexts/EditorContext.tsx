import { createContext, useContext } from "react";
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
	updateComment: (id: string, content: string) => void;

	showSentComments: boolean;
	toggleShowSentComments: () => void;

	showInlineComments: boolean;
	toggleShowInlineComments: () => void;

	rootPath: string;
	onStageHunk?: (repoPath: string, patch: string) => Promise<void>;
	onGitChanged?: () => void;
	gitRefreshKey: number;

	theme?: Theme;
	fontSize?: number;
	onSearchOccurrences?: (text: string) => void;
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
