import type * as Monaco from "monaco-editor";
import { useCallback, useMemo, useRef } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { useMonacoDiffEditor } from "@/hooks/useMonacoDiffEditor";
import { useMonacoGutterEditor } from "@/hooks/useMonacoGutterEditor";
import type { ChangeGroup } from "@/lib/computeHunks";
import { cn } from "@/lib/utils";
import type { CommentRange, LineComment } from "@/types/comment";
import type { DiffMode, Theme } from "@/types/settings";

export type { CommentRange } from "@/types/comment";
export type { DiffBase, DiffMode } from "@/types/settings";

interface RevealLine {
	line: number;
	key: number;
}

interface HunkCommentProps {
	changeGroups?: ChangeGroup[];
	commentRanges?: CommentRange[];
	onStageHunk?: (hunkIndex: number) => void;
	onUnstageHunk?: (hunkIndex: number) => void;
	onAddComment?: (
		lineNumber: number,
		content: string,
		endLine?: number,
	) => void;
	getCommentsForLine?: (lineNumber: number) => LineComment[];
	revealLine?: RevealLine;
	theme?: Theme;
}

interface NavigationHandlers {
	onSearchOccurrences?: (text: string) => void;
}

function useEditorContextMenu(
	editorRef: React.RefObject<Monaco.editor.ICodeEditor | null>,
	navigation?: NavigationHandlers,
) {
	const handleCopy = useCallback(async () => {
		const editor = editorRef.current;
		if (!editor) return;
		const selection = editor.getSelection();
		if (!selection) return;
		const model = editor.getModel();
		if (!model) return;
		const text = model.getValueInRange(selection);
		if (text) {
			await navigator.clipboard.writeText(text);
		}
	}, [editorRef]);

	const handleCut = useCallback(async () => {
		const editor = editorRef.current;
		if (!editor) return;
		const selection = editor.getSelection();
		if (!selection) return;
		const model = editor.getModel();
		if (!model) return;
		const text = model.getValueInRange(selection);
		if (text) {
			await navigator.clipboard.writeText(text);
			editor.executeEdits("cut", [{ range: selection, text: "" }]);
		}
	}, [editorRef]);

	const handlePaste = useCallback(async () => {
		const editor = editorRef.current;
		if (!editor) return;
		const selection = editor.getSelection();
		if (!selection) return;
		const text = await navigator.clipboard.readText();
		editor.executeEdits("paste", [{ range: selection, text }]);
	}, [editorRef]);

	const handleUndo = useCallback(() => {
		editorRef.current?.trigger("contextMenu", "undo", null);
	}, [editorRef]);

	const handleRedo = useCallback(() => {
		editorRef.current?.trigger("contextMenu", "redo", null);
	}, [editorRef]);

	const handleSelectAll = useCallback(() => {
		const editor = editorRef.current;
		if (!editor) return;
		const model = editor.getModel();
		if (!model) return;
		editor.setSelection(model.getFullModelRange());
	}, [editorRef]);

	const getWordAtCursor = useCallback(() => {
		const editor = editorRef.current;
		if (!editor) return null;
		const position = editor.getPosition();
		if (!position) return null;
		const model = editor.getModel();
		if (!model) return null;
		const selection = editor.getSelection();
		if (selection && !selection.isEmpty()) {
			return model.getValueInRange(selection);
		}
		const word = model.getWordAtPosition(position);
		return word?.word ?? null;
	}, [editorRef]);

	const handleGoToDefinition = useCallback(() => {
		editorRef.current?.trigger(
			"contextMenu",
			"editor.action.revealDefinition",
			null,
		);
	}, [editorRef]);

	const handleFindReferences = useCallback(() => {
		editorRef.current?.trigger(
			"contextMenu",
			"editor.action.goToReferences",
			null,
		);
	}, [editorRef]);

	const handleSearchOccurrences = useCallback(() => {
		const word = getWordAtCursor();
		if (!word || !navigation?.onSearchOccurrences) return;
		navigation.onSearchOccurrences(word);
	}, [getWordAtCursor, navigation]);

	return {
		handleCopy,
		handleCut,
		handlePaste,
		handleUndo,
		handleRedo,
		handleSelectAll,
		handleGoToDefinition,
		handleFindReferences,
		handleSearchOccurrences,
		hasNavigation: true,
	};
}

function EditorContextMenuContent({
	actions,
}: {
	actions: ReturnType<typeof useEditorContextMenu>;
}) {
	return (
		<ContextMenuContent className="w-56">
			{actions.hasNavigation && (
				<>
					<ContextMenuItem onClick={actions.handleGoToDefinition}>
						定義へ移動
					</ContextMenuItem>
					<ContextMenuItem onClick={actions.handleFindReferences}>
						参照を検索
					</ContextMenuItem>
					<ContextMenuSeparator />
					<ContextMenuItem onClick={actions.handleSearchOccurrences}>
						すべての出現箇所を検索
					</ContextMenuItem>
					<ContextMenuSeparator />
				</>
			)}
			<ContextMenuItem onClick={actions.handleCopy}>コピー</ContextMenuItem>
			<ContextMenuItem onClick={actions.handleCut}>切り取り</ContextMenuItem>
			<ContextMenuItem onClick={actions.handlePaste}>貼り付け</ContextMenuItem>
			<ContextMenuSeparator />
			<ContextMenuItem onClick={actions.handleUndo}>元に戻す</ContextMenuItem>
			<ContextMenuItem onClick={actions.handleRedo}>やり直し</ContextMenuItem>
			<ContextMenuSeparator />
			<ContextMenuItem onClick={actions.handleSelectAll}>
				全選択
			</ContextMenuItem>
		</ContextMenuContent>
	);
}

function GutterEditor({
	originalContent,
	modifiedContent,
	language,
	onContentChange,
	fontSize,
	changeGroups,
	commentRanges,
	onStageHunk,
	onUnstageHunk,
	onAddComment,
	getCommentsForLine,
	revealLine,
	theme,
	navigation,
	filePath,
	readOnly,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	onContentChange?: (content: string) => void;
	fontSize?: number;
	filePath?: string;
	navigation?: NavigationHandlers;
	readOnly?: boolean;
} & HunkCommentProps) {
	const containerRef = useRef<HTMLDivElement>(null);

	const { editorRef } = useMonacoGutterEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
		language,
		onContentChange,
		fontSize,
		filePath,
		changeGroups,
		commentRanges,
		onStageHunk,
		onUnstageHunk,
		onAddComment,
		getCommentsForLine,
		revealLine,
		theme,
		readOnly,
	});

	const actions = useEditorContextMenu(editorRef, navigation);

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div ref={containerRef} className="h-full w-full" />
			</ContextMenuTrigger>
			<EditorContextMenuContent actions={actions} />
		</ContextMenu>
	);
}

function DiffEditor({
	originalContent,
	modifiedContent,
	language,
	renderSideBySide,
	onContentChange,
	fontSize,
	changeGroups,
	commentRanges,
	onStageHunk,
	onUnstageHunk,
	onAddComment,
	getCommentsForLine,
	revealLine,
	theme,
	navigation,
	filePath,
	readOnly,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	renderSideBySide: boolean;
	onContentChange?: (content: string) => void;
	fontSize?: number;
	filePath?: string;
	navigation?: NavigationHandlers;
	readOnly?: boolean;
} & HunkCommentProps) {
	const containerRef = useRef<HTMLDivElement>(null);

	const { diffEditorRef } = useMonacoDiffEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
		language,
		renderSideBySide,
		onContentChange,
		fontSize,
		filePath,
		changeGroups,
		commentRanges,
		onStageHunk,
		onUnstageHunk,
		onAddComment,
		getCommentsForLine,
		revealLine,
		theme,
		readOnly,
	});

	const modifiedEditorProxy = useMemo<
		React.RefObject<Monaco.editor.ICodeEditor | null>
	>(
		() => ({
			get current() {
				return diffEditorRef.current?.getModifiedEditor() ?? null;
			},
		}),
		[diffEditorRef],
	);

	const actions = useEditorContextMenu(modifiedEditorProxy, navigation);

	return (
		<ContextMenu>
			<ContextMenuTrigger asChild>
				<div ref={containerRef} className="h-full w-full" />
			</ContextMenuTrigger>
			<EditorContextMenuContent actions={actions} />
		</ContextMenu>
	);
}

interface MonacoDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	language?: string;
	className?: string;
	diffMode?: DiffMode;
	onContentChange?: (content: string) => void;
	fontSize?: number;
	changeGroups?: ChangeGroup[];
	commentRanges?: CommentRange[];
	onStageHunk?: (hunkIndex: number) => void;
	onUnstageHunk?: (hunkIndex: number) => void;
	onAddComment?: (
		lineNumber: number,
		content: string,
		endLine?: number,
	) => void;
	getCommentsForLine?: (lineNumber: number) => LineComment[];
	revealLine?: RevealLine;
	theme?: Theme;
	filePath?: string;
	onSearchOccurrences?: (text: string) => void;
	readOnly?: boolean;
}

export function MonacoDiffViewer({
	originalContent,
	modifiedContent,
	language = "typescript",
	className,
	diffMode = "split",
	onContentChange,
	fontSize,
	changeGroups,
	commentRanges,
	onStageHunk,
	onUnstageHunk,
	onAddComment,
	getCommentsForLine,
	revealLine,
	theme,
	filePath,
	onSearchOccurrences,
	readOnly,
}: MonacoDiffViewerProps) {
	const hunkCommentProps: HunkCommentProps = {
		changeGroups,
		commentRanges,
		onStageHunk,
		onUnstageHunk,
		onAddComment,
		getCommentsForLine,
		revealLine,
		theme,
	};

	const navigation: NavigationHandlers | undefined = onSearchOccurrences
		? { onSearchOccurrences }
		: undefined;

	return (
		<div className={cn("h-full w-full bg-background", className)}>
			{diffMode === "gutter" && (
				<GutterEditor
					originalContent={originalContent}
					modifiedContent={modifiedContent}
					language={language}
					onContentChange={onContentChange}
					fontSize={fontSize}
					filePath={filePath}
					navigation={navigation}
					readOnly={readOnly}
					{...hunkCommentProps}
				/>
			)}
			{diffMode === "inline" && (
				<DiffEditor
					originalContent={originalContent}
					modifiedContent={modifiedContent}
					language={language}
					renderSideBySide={false}
					onContentChange={onContentChange}
					fontSize={fontSize}
					filePath={filePath}
					navigation={navigation}
					readOnly={readOnly}
					{...hunkCommentProps}
				/>
			)}
			{diffMode === "split" && (
				<DiffEditor
					originalContent={originalContent}
					modifiedContent={modifiedContent}
					language={language}
					renderSideBySide={true}
					onContentChange={onContentChange}
					fontSize={fontSize}
					filePath={filePath}
					navigation={navigation}
					readOnly={readOnly}
					{...hunkCommentProps}
				/>
			)}
		</div>
	);
}
