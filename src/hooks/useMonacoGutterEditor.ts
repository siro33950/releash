import { loader } from "@monaco-editor/react";
import { diffLines } from "diff";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef, useState } from "react";
import {
	type CommentThreadZone,
	createCommentThread,
} from "@/lib/commentThreadWidget";
import type { ChangeGroup } from "@/lib/computeHunks";
import {
	DIFF_ADDED_COLOR,
	DIFF_DELETED_COLOR,
	defaultEditorOptions,
	disableBuiltinDiagnostics,
	getMonacoThemeName,
	MONACO_DARK_THEME_NAME,
	MONACO_LIGHT_THEME_NAME,
	monacoLightTheme,
	monacoTheme,
} from "@/lib/monaco-config";
import type { CommentRange } from "@/types/comment";
import type { Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";

interface RevealLine {
	line: number;
	key: number;
	openThread?: boolean;
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
	onAddEntry?: (threadId: string, content: string) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onUpdateEntry?: (threadId: string, entryId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
	getThreadsForLine?: (lineNumber: number) => Thread[];
	revealLine?: RevealLine;
	theme?: Theme;
	readOnly?: boolean;
}

interface DiffResult {
	added: number[];
	deleted: number[];
}

export function computeDiff(original: string, modified: string): DiffResult {
	const changes = diffLines(original, modified);
	const added: number[] = [];
	const deletedLines: number[] = [];

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
			const deletedMarkerLine =
				lineNumber <= modifiedLineCount ? lineNumber : modifiedLineCount;
			if (deletedMarkerLine > 0) {
				deletedLines.push(deletedMarkerLine);
			}
		} else {
			lineNumber += lines;
		}
	}

	const addedSet = new Set(added);
	const filteredDeleted = deletedLines.filter((l) => !addedSet.has(l));

	return { added, deleted: filteredDeleted };
}

export function createDiffDecorations(
	diff: DiffResult,
	monaco: typeof Monaco,
): Monaco.editor.IModelDeltaDecoration[] {
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
	for (const line of diff.deleted) {
		decorations.push({
			range: new monaco.Range(line, 1, line, 1),
			options: {
				isWholeLine: true,
				glyphMarginClassName: "gutter-deleted",
				overviewRuler: {
					color: DIFF_DELETED_COLOR,
					position: monaco.editor.OverviewRulerLane.Full,
				},
			},
		});
	}
	return decorations;
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
		onAddEntry,
		onDeleteThread,
		onResolveThread,
		onUpdateEntry,
		onCopyThread,
		getThreadsForLine,
		revealLine,
		theme,
		readOnly,
	} = options;

	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const modelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const [editorReady, setEditorReady] = useState(false);
	const intersectionObserverRef = useRef<IntersectionObserver | null>(null);
	const decorationsRef = useRef<string[]>([]);
	const commentDecorationsRef = useRef<string[]>([]);
	const originalValueRef = useRef(originalValue);
	const modifiedValueRef = useRef(modifiedValue);
	const onContentChangeRef = useRef(onContentChange);
	const fontSizeRef = useRef(fontSize);
	const onAddCommentRef = useRef(onAddComment);
	const onAddEntryRef = useRef(onAddEntry);
	const onDeleteThreadRef = useRef(onDeleteThread);
	const onResolveThreadRef = useRef(onResolveThread);
	const onUpdateEntryRef = useRef(onUpdateEntry);
	const onCopyThreadRef = useRef(onCopyThread);
	const getThreadsForLineRef = useRef(getThreadsForLine);
	const commentInputWidgetRef = useRef<CommentThreadZone | null>(null);
	const openWidgetInfoRef = useRef<{
		threadId: string;
		lineNumber: number;
	} | null>(null);
	const dragStartLineRef = useRef<number | null>(null);
	const dragRangeDecorationsRef = useRef<string[]>([]);
	const hoverLineRef = useRef<number | null>(null);
	const hoverDecorationsRef = useRef<string[]>([]);
	const isProgrammaticUpdateRef = useRef(false);
	const pendingRevealRef = useRef<(() => void) | null>(null);
	const themeRef = useRef(theme);
	const commentRangesRef = useRef(commentRanges);
	originalValueRef.current = originalValue;
	modifiedValueRef.current = modifiedValue;
	onContentChangeRef.current = onContentChange;
	fontSizeRef.current = fontSize;
	onAddCommentRef.current = onAddComment;
	onAddEntryRef.current = onAddEntry;
	onDeleteThreadRef.current = onDeleteThread;
	onResolveThreadRef.current = onResolveThread;
	onUpdateEntryRef.current = onUpdateEntry;
	onCopyThreadRef.current = onCopyThread;
	getThreadsForLineRef.current = getThreadsForLine;
	themeRef.current = theme;
	commentRangesRef.current = commentRanges;

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
			const editor = monaco.editor.create(
				container,
				{
					...defaultEditorOptions,
					model,
					theme: themeName,
					glyphMargin: true,
					...(fontSizeRef.current != null && {
						fontSize: fontSizeRef.current,
					}),
					...(readOnly != null && { readOnly }),
				},
				{
					textModelService: {
						createModelReference(uri: Monaco.Uri) {
							const m = monaco.editor.getModel(uri);
							if (!m)
								return Promise.reject(new Error(`Model not found: ${uri}`));
							return Promise.resolve({
								object: { textEditorModel: m },
								dispose() {},
							});
						},
						registerTextModelContentProvider() {
							return { dispose() {} };
						},
					},
				},
			);

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
				const decorations = createDiffDecorations(diff, monaco);
				decorationsRef.current = editor.deltaDecorations(
					decorationsRef.current,
					decorations,
				);
			};

			updateDecorations();

			editor.onDidChangeModelContent(() => {
				updateDecorations();
				if (isProgrammaticUpdateRef.current) return;
				onContentChangeRef.current?.(editor.getValue());
			});

			const openCommentWidget = (
				ed: Monaco.editor.ICodeEditor,
				lineNum: number,
				endLine?: number,
			) => {
				if (commentInputWidgetRef.current) {
					commentInputWidgetRef.current.dispose();
					commentInputWidgetRef.current = null;
					openWidgetInfoRef.current = null;
				}

				const existingThreads = getThreadsForLineRef.current?.(lineNum) ?? [];
				const thread: Thread = existingThreads[0] ?? {
					id: "",
					filePath: "",
					lineNumber: lineNum,
					...(endLine != null && { endLine }),
					entries: [],
					resolved: false,
					createdAt: Date.now(),
				};
				const isNew = existingThreads.length === 0;

				const zone = createCommentThread(ed, {
					thread,
					onSubmit: (content) => {
						if (isNew) {
							onAddCommentRef.current?.(lineNum, content, endLine);
							zone.dispose();
							commentInputWidgetRef.current = null;
							openWidgetInfoRef.current = null;
							ed.focus();
						} else {
							onAddEntryRef.current?.(thread.id, content);
							const textarea = zone.domNode.querySelector<HTMLTextAreaElement>(
								".comment-thread-textarea",
							);
							if (textarea) textarea.value = "";
						}
					},
					onCancel: () => {
						zone.dispose();
						commentInputWidgetRef.current = null;
						openWidgetInfoRef.current = null;
						ed.focus();
					},
					onDeleteThread: (threadId) => onDeleteThreadRef.current?.(threadId),
					onResolveThread: (threadId) => onResolveThreadRef.current?.(threadId),
					onUpdateEntry: (threadId, entryId, content) =>
						onUpdateEntryRef.current?.(threadId, entryId, content),
					onCopyThread: (t) => onCopyThreadRef.current?.(t),
				});
				commentInputWidgetRef.current = zone;
				openWidgetInfoRef.current = {
					threadId: thread.id,
					lineNumber: lineNum,
				};
			};

			editor.onMouseDown((e: Monaco.editor.IEditorMouseEvent) => {
				if (
					e.target.type ===
						monaco.editor.MouseTargetType.GUTTER_LINE_DECORATIONS ||
					e.target.type === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS
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
					const hasComment =
						lineNum != null &&
						commentRangesRef.current?.some(
							(r) => lineNum >= r.start && lineNum <= (r.end ?? r.start),
						);
					hoverDecorationsRef.current = editor.deltaDecorations(
						hoverDecorationsRef.current,
						lineNum != null && !hasComment
							? [
									{
										range: new monaco.Range(lineNum, 1, lineNum, 1),
										options: {
											lineNumberClassName: "comment-hover-margin",
										},
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

			const intersectionObserver = new IntersectionObserver((entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					requestAnimationFrame(() => {
						editorRef.current?.layout();
						const pending = pendingRevealRef.current;
						if (pending) {
							pendingRevealRef.current = null;
							pending();
						}
					});
				}
			});
			intersectionObserver.observe(container);
			intersectionObserverRef.current = intersectionObserver;
		};

		initEditor().catch((error) => {
			console.error("Failed to initialize Monaco Gutter Editor:", error);
		});

		return () => {
			isMounted = false;
			pendingRevealRef.current = null;
			commentInputWidgetRef.current?.dispose();
			commentInputWidgetRef.current = null;
			openWidgetInfoRef.current = null;
			intersectionObserverRef.current?.disconnect();
			intersectionObserverRef.current = null;
			editorRef.current?.dispose();
			editorRef.current = null;
			modelRef.current?.dispose();
			modelRef.current = null;
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

			isProgrammaticUpdateRef.current = true;
			try {
				editor.setValue(modifiedValue);
			} finally {
				isProgrammaticUpdateRef.current = false;
			}

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
		const decorations = createDiffDecorations(diff, monaco);
		decorationsRef.current = editor.deltaDecorations(
			decorationsRef.current,
			decorations,
		);
	}, [originalValue]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady ensures editor/monaco refs are initialized before applying decorations
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const decorations: Monaco.editor.IModelDeltaDecoration[] = [];
		const seen = new Set<number>();

		for (const r of commentRanges ?? []) {
			decorations.push({
				range: new monaco.Range(r.start, 1, r.end ?? r.start, 1),
				options: {
					isWholeLine: true,
					linesDecorationsClassName: "comment-marker",
				},
			});
			if (!seen.has(r.start)) {
				seen.add(r.start);
				decorations.push({
					range: new monaco.Range(r.start, 1, r.start, 1),
					options: {
						lineNumberClassName: "comment-marker-margin",
					},
				});
			}
		}

		commentDecorationsRef.current = editor.deltaDecorations(
			commentDecorationsRef.current,
			decorations,
		);
	}, [commentRanges, editorReady]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: containerRef.current is a ref that doesn't trigger re-renders; visibility is handled by IntersectionObserver + pendingRevealRef
	useEffect(() => {
		if (!editorReady || !revealLine) return;
		const editor = editorRef.current;
		if (!editor) return;

		const applyReveal = () => {
			editor.layout();
			editor.revealLineInCenter(revealLine.line);
			editor.setPosition({ lineNumber: revealLine.line, column: 1 });

			if (revealLine.openThread) {
				if (commentInputWidgetRef.current) {
					commentInputWidgetRef.current.dispose();
					commentInputWidgetRef.current = null;
					openWidgetInfoRef.current = null;
				}
				const existingThreads =
					getThreadsForLineRef.current?.(revealLine.line) ?? [];
				if (existingThreads.length > 0) {
					const thread = existingThreads[0];
					const zone = createCommentThread(editor, {
						thread,
						onSubmit: (content) => {
							onAddEntryRef.current?.(thread.id, content);
							const textarea = zone.domNode.querySelector<HTMLTextAreaElement>(
								".comment-thread-textarea",
							);
							if (textarea) textarea.value = "";
						},
						onCancel: () => {
							zone.dispose();
							commentInputWidgetRef.current = null;
							openWidgetInfoRef.current = null;
							editor.focus();
						},
						onDeleteThread: (threadId) => onDeleteThreadRef.current?.(threadId),
						onResolveThread: (threadId) =>
							onResolveThreadRef.current?.(threadId),
						onUpdateEntry: (threadId, entryId, content) =>
							onUpdateEntryRef.current?.(threadId, entryId, content),
						onCopyThread: (t) => onCopyThreadRef.current?.(t),
					});
					commentInputWidgetRef.current = zone;
					openWidgetInfoRef.current = {
						threadId: thread.id,
						lineNumber: revealLine.line,
					};
				}
			}
		};

		const container = containerRef.current;
		if (container?.offsetParent !== null) {
			applyReveal();
		} else {
			pendingRevealRef.current = applyReveal;
		}
	}, [revealLine, editorReady]);

	useEffect(() => {
		const monaco = monacoRef.current;
		if (!monaco || !theme) return;
		monaco.editor.setTheme(getMonacoThemeName(theme));
	}, [theme]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: getThreadsForLine identity changes when threads update; used intentionally as a trigger to re-render the open widget
	useEffect(() => {
		const widget = commentInputWidgetRef.current;
		const info = openWidgetInfoRef.current;
		if (!widget || !info) return;
		const threads = getThreadsForLineRef.current?.(info.lineNumber) ?? [];
		const thread = threads.find((t) => t.id === info.threadId);
		if (thread) {
			widget.update({ thread });
		} else if (info.threadId !== "") {
			widget.dispose();
			commentInputWidgetRef.current = null;
			openWidgetInfoRef.current = null;
		}
	}, [getThreadsForLine]);

	return {
		editorRef,
		monacoRef,
	};
}
