import type * as Monaco from "monaco-editor";
import { useCallback, useMemo, useRef } from "react";
import {
	ContextMenu,
	ContextMenuContent,
	ContextMenuItem,
	ContextMenuSeparator,
	ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { ChangeGroup } from "@/lib/computeHunks";
import { useMonacoDiffEditor } from "@/hooks/useMonacoDiffEditor";
import { useMonacoGutterEditor } from "@/hooks/useMonacoGutterEditor";
import { cn } from "@/lib/utils";
import type { CommentRange, LineComment } from "@/types/comment";
import type { DiffMode, Theme } from "@/types/settings";

export type { CommentRange } from "@/types/comment";
export type { DiffMode, DiffBase } from "@/types/settings";

interface RevealLine {
	line: number;
	key: number;
}

interface HunkCommentProps {
	changeGroups?: ChangeGroup[];
	commentRanges?: CommentRange[];
	onStageHunk?: (hunkIndex: number) => void;
	onUnstageHunk?: (hunkIndex: number) => void;
	onAddComment?: (lineNumber: number, content: string, endLine?: number) => void;
	getCommentsForLine?: (lineNumber: number) => LineComment[];
	revealLine?: RevealLine;
	theme?: Theme;
}

function useEditorContextMenu(
	editorRef: React.RefObject<Monaco.editor.ICodeEditor | null>,
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

	return {
		handleCopy,
		handleCut,
		handlePaste,
		handleUndo,
		handleRedo,
		handleSelectAll,
	};
}

function EditorContextMenuContent({
	actions,
}: {
	actions: ReturnType<typeof useEditorContextMenu>;
}) {
	return (
		<ContextMenuContent className="w-56">
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
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	onContentChange?: (content: string) => void;
	fontSize?: number;
} & HunkCommentProps) {
	const containerRef = useRef<HTMLDivElement>(null);

	const { editorRef } = useMonacoGutterEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
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
	});

	const actions = useEditorContextMenu(editorRef);

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
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	renderSideBySide: boolean;
	onContentChange?: (content: string) => void;
	fontSize?: number;
} & HunkCommentProps) {
	const containerRef = useRef<HTMLDivElement>(null);

	const { diffEditorRef } = useMonacoDiffEditor(containerRef, {
		originalValue: originalContent,
		modifiedValue: modifiedContent,
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

	const actions = useEditorContextMenu(modifiedEditorProxy);

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
	onAddComment?: (lineNumber: number, content: string, endLine?: number) => void;
	getCommentsForLine?: (lineNumber: number) => LineComment[];
	revealLine?: RevealLine;
	theme?: Theme;
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

	return (
		<div className={cn("h-full w-full bg-background", className)}>
			{diffMode === "gutter" && (
				<GutterEditor
					originalContent={originalContent}
					modifiedContent={modifiedContent}
					language={language}
					onContentChange={onContentChange}
					fontSize={fontSize}
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
					{...hunkCommentProps}
				/>
			)}
		</div>
	);
}
