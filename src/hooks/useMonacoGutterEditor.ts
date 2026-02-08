import { loader } from "@monaco-editor/react";
import { diffLines } from "diff";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef, useState } from "react";
import {
	createCommentPeekWidget,
	type MonacoContentWidget,
} from "@/lib/commentPeekWidget";
import type { ChangeGroup } from "@/lib/computeHunks";
import {
	DIFF_ADDED_COLOR,
	DIFF_MODIFIED_COLOR,
	defaultEditorOptions,
	disableBuiltinDiagnostics,
	getMonacoThemeName,
	MONACO_DARK_THEME_NAME,
	MONACO_LIGHT_THEME_NAME,
	monacoLightTheme,
	monacoTheme,
} from "@/lib/monaco-config";
import type { CommentRange, LineComment } from "@/types/comment";
import type { Theme } from "@/types/settings";

interface RevealLine {
	line: number;
	key: number;
}

interface UseMonacoGutterEditorOptions {
	originalValue: string;
	modifiedValue: string;
	language?: string;
	onContentChange?: (content: string) => void;
	fontSize?: number;
	filePath?: string;
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
	readOnly?: boolean;
}

interface DiffResult {
	added: number[];
	modified: number[];
}

export function computeDiff(original: string, modified: string): DiffResult {
	const changes = diffLines(original, modified);
	const added: number[] = [];
	const modified_lines: number[] = [];

	const modifiedLineCount =
		modified === ""
			? 0
			: modified.split("\n").length - (modified.endsWith("\n") ? 1 : 0);

	let lineNumber = 1;

	for (const change of changes) {
		const lines = change.count || 0;

		if (change.added) {
			for (let i = 0; i < lines; i++) {
				added.push(lineNumber + i);
			}
			lineNumber += lines;
		} else if (change.removed) {
			if (lines > 0 && lineNumber <= modifiedLineCount) {
				modified_lines.push(lineNumber);
			}
		} else {
			lineNumber += lines;
		}
	}

	return { added, modified: modified_lines };
}

export function useMonacoGutterEditor(
	containerRef: RefObject<HTMLDivElement | null>,
	options: UseMonacoGutterEditorOptions,
) {
	const {
		originalValue,
		modifiedValue,
		language = "typescript",
		onContentChange,
		fontSize,
		filePath,
		commentRanges,
		onAddComment,
		getCommentsForLine,
		revealLine,
		theme,
		readOnly,
	} = options;

	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const modelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const [, setEditorReady] = useState(false);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const decorationsRef = useRef<string[]>([]);
	const commentDecorationsRef = useRef<string[]>([]);
	const originalValueRef = useRef(originalValue);
	const modifiedValueRef = useRef(modifiedValue);
	const onContentChangeRef = useRef(onContentChange);
	const fontSizeRef = useRef(fontSize);
	const onAddCommentRef = useRef(onAddComment);
	const getCommentsForLineRef = useRef(getCommentsForLine);
	const commentInputWidgetRef = useRef<MonacoContentWidget | null>(null);
	const dragStartLineRef = useRef<number | null>(null);
	const dragRangeDecorationsRef = useRef<string[]>([]);
	const hoverLineRef = useRef<number | null>(null);
	const hoverDecorationsRef = useRef<string[]>([]);
	const themeRef = useRef(theme);
	originalValueRef.current = originalValue;
	modifiedValueRef.current = modifiedValue;
	onContentChangeRef.current = onContentChange;
	fontSizeRef.current = fontSize;
	onAddCommentRef.current = onAddComment;
	getCommentsForLineRef.current = getCommentsForLine;
	themeRef.current = theme;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const initEditor = async () => {
			const monaco = await loader.init();

			if (!isMounted) return;

			monacoRef.current = monaco;
			disableBuiltinDiagnostics(monaco);

			monaco.editor.defineTheme(MONACO_DARK_THEME_NAME, monacoTheme);
			monaco.editor.defineTheme(MONACO_LIGHT_THEME_NAME, monacoLightTheme);
			const themeName = getMonacoThemeName(themeRef.current ?? "dark");
			monaco.editor.setTheme(themeName);

			const modelUri = filePath ? monaco.Uri.file(filePath) : undefined;
			const existingModel = modelUri ? monaco.editor.getModel(modelUri) : null;
			if (existingModel) {
				existingModel.dispose();
			}
			const model = monaco.editor.createModel(
				modifiedValueRef.current,
				language,
				modelUri,
			);
			const editor = monaco.editor.create(container, {
				...defaultEditorOptions,
				model,
				theme: themeName,
				glyphMargin: true,
				...(fontSizeRef.current != null && { fontSize: fontSizeRef.current }),
				...(readOnly != null && { readOnly }),
			});

			if (!isMounted) {
				editor.dispose();
				model.dispose();
				return;
			}

			modelRef.current = model;
			editorRef.current = editor;
			setEditorReady(true);

			const updateDecorations = () => {
				const currentValue = editor.getValue();
				const diff = computeDiff(originalValueRef.current, currentValue);
				const decorations: Monaco.editor.IModelDeltaDecoration[] = [];

				for (const line of diff.added) {
					decorations.push({
						range: new monaco.Range(line, 1, line, 1),
						options: {
							isWholeLine: true,
							glyphMarginClassName: "gutter-added",
							overviewRuler: {
								color: DIFF_ADDED_COLOR,
								position: monaco.editor.OverviewRulerLane.Full,
							},
						},
					});
				}

				for (const line of diff.modified) {
					decorations.push({
						range: new monaco.Range(line, 1, line, 1),
						options: {
							isWholeLine: true,
							glyphMarginClassName: "gutter-modified",
							overviewRuler: {
								color: DIFF_MODIFIED_COLOR,
								position: monaco.editor.OverviewRulerLane.Full,
							},
						},
					});
				}

				decorationsRef.current = editor.deltaDecorations(
					decorationsRef.current,
					decorations,
				);
			};

			updateDecorations();

			editor.onDidChangeModelContent(() => {
				updateDecorations();
				onContentChangeRef.current?.(editor.getValue());
			});

			const openCommentWidget = (
				ed: Monaco.editor.ICodeEditor,
				lineNum: number,
				endLine?: number,
			) => {
				if (commentInputWidgetRef.current) {
					ed.removeContentWidget(commentInputWidgetRef.current);
					commentInputWidgetRef.current = null;
				}

				const existing = getCommentsForLineRef.current?.(lineNum) ?? [];
				const widget = createCommentPeekWidget(monaco, {
					lineNumber: lineNum,
					endLine,
					existingComments: existing,
					onSubmit: (content) => {
						onAddCommentRef.current?.(lineNum, content, endLine);
						ed.removeContentWidget(widget);
						commentInputWidgetRef.current = null;
						ed.focus();
					},
					onCancel: () => {
						ed.removeContentWidget(widget);
						commentInputWidgetRef.current = null;
						ed.focus();
					},
				});
				commentInputWidgetRef.current = widget;
				ed.addContentWidget(widget);
			};

			editor.onMouseDown((e: Monaco.editor.IEditorMouseEvent) => {
				if (
					e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
				) {
					const lineNum = e.target.position?.lineNumber;
					if (!lineNum) return;
					dragStartLineRef.current = lineNum;
				}
			});

			editor.onMouseMove((e: Monaco.editor.IEditorMouseEvent) => {
				const lineNum = e.target.position?.lineNumber ?? null;

				if (dragStartLineRef.current != null) {
					if (lineNum) {
						const startLine = Math.min(dragStartLineRef.current, lineNum);
						const endLine = Math.max(dragStartLineRef.current, lineNum);
						dragRangeDecorationsRef.current = editor.deltaDecorations(
							dragRangeDecorationsRef.current,
							startLine !== endLine
								? [
										{
											range: new monaco.Range(startLine, 1, endLine, 1),
											options: {
												isWholeLine: true,
												className: "comment-range-highlight",
											},
										},
									]
								: [],
						);
					}
					if (hoverLineRef.current != null) {
						hoverLineRef.current = null;
						hoverDecorationsRef.current = editor.deltaDecorations(
							hoverDecorationsRef.current,
							[],
						);
					}
					return;
				}

				if (lineNum !== hoverLineRef.current) {
					hoverLineRef.current = lineNum;
					hoverDecorationsRef.current = editor.deltaDecorations(
						hoverDecorationsRef.current,
						lineNum != null
							? [
									{
										range: new monaco.Range(lineNum, 1, lineNum, 1),
										options: { glyphMarginClassName: "comment-hover-icon" },
									},
								]
							: [],
					);
				}
			});

			editor.onMouseUp((e: Monaco.editor.IEditorMouseEvent) => {
				if (dragStartLineRef.current == null) return;

				const startLine = dragStartLineRef.current;
				dragStartLineRef.current = null;

				dragRangeDecorationsRef.current = editor.deltaDecorations(
					dragRangeDecorationsRef.current,
					[],
				);

				const selection = editor.getSelection();
				let lo: number;
				let hi: number;
				if (
					selection &&
					!selection.isEmpty() &&
					selection.startLineNumber !== selection.endLineNumber
				) {
					lo = Math.min(selection.startLineNumber, startLine);
					hi = Math.max(selection.endLineNumber, startLine);
				} else {
					const lineNum = e.target.position?.lineNumber ?? startLine;
					lo = Math.min(startLine, lineNum);
					hi = Math.max(startLine, lineNum);
				}

				editor.setSelection(new monaco.Selection(lo, 1, lo, 1));

				if (lo === hi) {
					openCommentWidget(editor, lo);
				} else {
					openCommentWidget(editor, lo, hi);
				}
			});

			editor.addAction({
				id: "releash.addComment",
				label: "Add Comment",
				keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyK],
				run: (ed: Monaco.editor.ICodeEditor) => {
					const position = ed.getPosition();
					if (!position) return;
					const selection = ed.getSelection();
					if (
						selection &&
						!selection.isEmpty() &&
						selection.startLineNumber !== selection.endLineNumber
					) {
						openCommentWidget(
							ed,
							selection.startLineNumber,
							selection.endLineNumber,
						);
					} else {
						openCommentWidget(ed, position.lineNumber);
					}
				},
			});

			const resizeObserver = new ResizeObserver(() => {
				editorRef.current?.layout();
			});
			resizeObserver.observe(container);
			resizeObserverRef.current = resizeObserver;
		};

		initEditor().catch((error) => {
			console.error("Failed to initialize Monaco Gutter Editor:", error);
		});

		return () => {
			isMounted = false;
			resizeObserverRef.current?.disconnect();
			editorRef.current?.dispose();
			modelRef.current?.dispose();
		};
	}, [containerRef, language, filePath, readOnly]);

	useEffect(() => {
		const editor = editorRef.current;
		if (!editor || fontSize == null) return;
		editor.updateOptions({ fontSize });
	}, [fontSize]);

	useEffect(() => {
		const editor = editorRef.current;
		if (!editor) return;

		const currentValue = editor.getValue();
		if (currentValue !== modifiedValue) {
			const scrollTop = editor.getScrollTop();
			const position = editor.getPosition();

			editor.setValue(modifiedValue);

			editor.setScrollTop(scrollTop);
			if (position) {
				editor.setPosition(position);
			}
		}
	}, [modifiedValue]);

	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const currentValue = editor.getValue();
		const diff = computeDiff(originalValue, currentValue);
		const decorations: Monaco.editor.IModelDeltaDecoration[] = [];

		for (const line of diff.added) {
			decorations.push({
				range: new monaco.Range(line, 1, line, 1),
				options: {
					isWholeLine: true,
					glyphMarginClassName: "gutter-added",
					overviewRuler: {
						color: "rgba(80, 200, 80, 0.8)",
						position: monaco.editor.OverviewRulerLane.Full,
					},
				},
			});
		}

		for (const line of diff.modified) {
			decorations.push({
				range: new monaco.Range(line, 1, line, 1),
				options: {
					isWholeLine: true,
					glyphMarginClassName: "gutter-modified",
					overviewRuler: {
						color: "rgba(80, 160, 240, 0.8)",
						position: monaco.editor.OverviewRulerLane.Full,
					},
				},
			});
		}

		decorationsRef.current = editor.deltaDecorations(
			decorationsRef.current,
			decorations,
		);
	}, [originalValue]);

	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco || !commentRanges) return;

		const decorations: Monaco.editor.IModelDeltaDecoration[] =
			commentRanges.map((r) => ({
				range: new monaco.Range(r.start, 1, r.end ?? r.start, 1),
				options: {
					isWholeLine: true,
					glyphMarginClassName: "comment-marker",
				},
			}));

		commentDecorationsRef.current = editor.deltaDecorations(
			commentDecorationsRef.current,
			decorations,
		);
	}, [commentRanges]);

	useEffect(() => {
		const editor = editorRef.current;
		if (!editor || !revealLine) return;
		editor.revealLineInCenter(revealLine.line);
		editor.setPosition({ lineNumber: revealLine.line, column: 1 });
	}, [revealLine]);

	useEffect(() => {
		const monaco = monacoRef.current;
		if (!monaco || !theme) return;
		monaco.editor.setTheme(getMonacoThemeName(theme));
	}, [theme]);

	return {
		editorRef,
		monacoRef,
	};
}
