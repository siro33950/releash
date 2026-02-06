import { loader } from "@monaco-editor/react";
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
	defaultDiffEditorOptions,
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

interface UseMonacoDiffEditorOptions {
	originalValue: string;
	modifiedValue: string;
	language?: string;
	renderSideBySide?: boolean;
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
}

interface HunkOverlay {
	domNode: HTMLDivElement;
	lineNumber: number;
}

function createHunkOverlaysFromLineChanges(
	container: HTMLDivElement,
	lineChanges: Monaco.editor.ILineChange[],
	groups: ChangeGroup[],
	onStageRef: React.RefObject<((idx: number) => void) | undefined>,
	onUnstageRef: React.RefObject<((idx: number) => void) | undefined>,
): HunkOverlay[] {
	const overlays: HunkOverlay[] = [];
	for (let i = 0; i < lineChanges.length; i++) {
		const change = lineChanges[i];
		const group = groups[i];
		if (!group) continue;

		const domNode = document.createElement("div");
		domNode.className = "hunk-widget";

		if (onStageRef.current) {
			const idx = group.groupIndex;
			const isStaged = group.isStaged === true;
			const hasUnstage = group.isStaged != null && onUnstageRef.current;

			const seg = document.createElement("div");
			seg.className = "hunk-segment";

			const stagedBtn = document.createElement("button");
			stagedBtn.textContent = "Staged";
			stagedBtn.className = isStaged
				? "hunk-seg-btn hunk-seg-active"
				: "hunk-seg-btn";
			stagedBtn.addEventListener("click", (e) => {
				e.stopPropagation();
				if (!isStaged) onStageRef.current?.(idx);
			});

			const unstagedBtn = document.createElement("button");
			unstagedBtn.textContent = "Unstaged";
			unstagedBtn.className = isStaged
				? "hunk-seg-btn"
				: "hunk-seg-btn hunk-seg-active";
			unstagedBtn.addEventListener("click", (e) => {
				e.stopPropagation();
				if (isStaged && hasUnstage) onUnstageRef.current?.(idx);
			});

			seg.appendChild(stagedBtn);
			seg.appendChild(unstagedBtn);
			domNode.appendChild(seg);
		}

		const endLine =
			change.modifiedEndLineNumber > 0
				? change.modifiedEndLineNumber + 1
				: change.modifiedStartLineNumber + 1;

		container.appendChild(domNode);
		overlays.push({ domNode, lineNumber: endLine });
	}
	return overlays;
}

function updateOverlayPositions(
	editor: Monaco.editor.ICodeEditor,
	overlays: HunkOverlay[],
) {
	const viewportHeight = editor.getLayoutInfo().height;
	for (const overlay of overlays) {
		const pos = editor.getScrolledVisiblePosition({
			lineNumber: overlay.lineNumber,
			column: 1,
		});
		if (pos) {
			overlay.domNode.style.top = `${pos.top}px`;
			overlay.domNode.style.display =
				pos.top >= -30 && pos.top < viewportHeight ? "" : "none";
		} else {
			overlay.domNode.style.display = "none";
		}
	}
}

export function useMonacoDiffEditor(
	containerRef: RefObject<HTMLDivElement | null>,
	options: UseMonacoDiffEditorOptions,
) {
	const {
		originalValue,
		modifiedValue,
		language = "typescript",
		renderSideBySide = true,
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
	} = options;

	const diffEditorRef = useRef<Monaco.editor.IStandaloneDiffEditor | null>(
		null,
	);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const [editorReady, setEditorReady] = useState(false);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const originalModelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const modifiedModelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const contentChangeListenerRef = useRef<Monaco.IDisposable | null>(null);
	const commentDecorationsRef = useRef<string[]>([]);
	const diffDecorationsRef = useRef<string[]>([]);
	const originalValueRef = useRef(originalValue);
	const modifiedValueRef = useRef(modifiedValue);
	const onContentChangeRef = useRef(onContentChange);
	const fontSizeRef = useRef(fontSize);
	const onAddCommentRef = useRef(onAddComment);
	const getCommentsForLineRef = useRef(getCommentsForLine);
	const onStageHunkRef = useRef(onStageHunk);
	const onUnstageHunkRef = useRef(onUnstageHunk);
	const hunkOverlayContainerRef = useRef<HTMLDivElement | null>(null);
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
	onStageHunkRef.current = onStageHunk;
	onUnstageHunkRef.current = onUnstageHunk;
	themeRef.current = theme;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const initDiffEditor = async () => {
			const monaco = await loader.init();

			if (!isMounted) return;

			monacoRef.current = monaco;
			disableBuiltinDiagnostics(monaco);

			monaco.editor.defineTheme(MONACO_DARK_THEME_NAME, monacoTheme);
			monaco.editor.defineTheme(MONACO_LIGHT_THEME_NAME, monacoLightTheme);
			const themeName = getMonacoThemeName(themeRef.current ?? "dark");
			monaco.editor.setTheme(themeName);

			const originalModel = monaco.editor.createModel(
				originalValueRef.current,
				language,
			);
			const modifiedUri = filePath ? monaco.Uri.file(filePath) : undefined;
			const existingModel = modifiedUri
				? monaco.editor.getModel(modifiedUri)
				: null;
			if (existingModel) {
				existingModel.dispose();
			}
			const modifiedModel = monaco.editor.createModel(
				modifiedValueRef.current,
				language,
				modifiedUri,
			);

			if (!isMounted) {
				originalModel.dispose();
				modifiedModel.dispose();
				return;
			}

			originalModelRef.current = originalModel;
			modifiedModelRef.current = modifiedModel;

			const diffEditor = monaco.editor.createDiffEditor(container, {
				...defaultDiffEditorOptions,
				renderSideBySide,
				theme: themeName,
				...(fontSizeRef.current != null && { fontSize: fontSizeRef.current }),
			});

			if (!isMounted) {
				diffEditor.dispose();
				originalModel.dispose();
				modifiedModel.dispose();
				return;
			}

			diffEditor.setModel({
				original: originalModel,
				modified: modifiedModel,
			});

			contentChangeListenerRef.current = diffEditor
				.getModifiedEditor()
				.onDidChangeModelContent(() => {
					onContentChangeRef.current?.(modifiedModel.getValue());
				});

			diffEditorRef.current = diffEditor;
			setEditorReady(true);

			const modifiedEditor = diffEditor.getModifiedEditor();

			const updateDiffDecorations = () => {
				const changes = diffEditor.getLineChanges();
				if (!changes) return;
				const decorations: Monaco.editor.IModelDeltaDecoration[] = [];
				for (const change of changes) {
					if (change.modifiedStartLineNumber <= change.modifiedEndLineNumber) {
						decorations.push({
							range: new monaco.Range(
								change.modifiedStartLineNumber,
								1,
								change.modifiedEndLineNumber,
								1,
							),
							options: {
								overviewRuler: {
									color:
										change.originalEndLineNumber === 0
											? DIFF_ADDED_COLOR
											: DIFF_MODIFIED_COLOR,
									position: monaco.editor.OverviewRulerLane.Full,
								},
							},
						});
					}
				}
				diffDecorationsRef.current = modifiedEditor.deltaDecorations(
					diffDecorationsRef.current,
					decorations,
				);
			};
			diffEditor.onDidUpdateDiff(updateDiffDecorations);

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

			modifiedEditor.onMouseDown((e: Monaco.editor.IEditorMouseEvent) => {
				if (
					e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
				) {
					const lineNum = e.target.position?.lineNumber;
					if (!lineNum) return;
					dragStartLineRef.current = lineNum;
				}
			});

			modifiedEditor.onMouseMove((e: Monaco.editor.IEditorMouseEvent) => {
				const lineNum = e.target.position?.lineNumber ?? null;

				if (dragStartLineRef.current != null) {
					if (lineNum) {
						const startLine = Math.min(dragStartLineRef.current, lineNum);
						const endLine = Math.max(dragStartLineRef.current, lineNum);
						dragRangeDecorationsRef.current = modifiedEditor.deltaDecorations(
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
						hoverDecorationsRef.current = modifiedEditor.deltaDecorations(
							hoverDecorationsRef.current,
							[],
						);
					}
					return;
				}

				if (lineNum !== hoverLineRef.current) {
					hoverLineRef.current = lineNum;
					hoverDecorationsRef.current = modifiedEditor.deltaDecorations(
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

			modifiedEditor.onMouseUp((e: Monaco.editor.IEditorMouseEvent) => {
				if (dragStartLineRef.current == null) return;

				const lineNum =
					e.target.position?.lineNumber ?? dragStartLineRef.current;
				const startLine = dragStartLineRef.current;
				dragStartLineRef.current = null;

				dragRangeDecorationsRef.current = modifiedEditor.deltaDecorations(
					dragRangeDecorationsRef.current,
					[],
				);

				const lo = Math.min(startLine, lineNum);
				const hi = Math.max(startLine, lineNum);

				if (lo === hi) {
					openCommentWidget(modifiedEditor, lo);
				} else {
					openCommentWidget(modifiedEditor, lo, hi);
				}
			});

			modifiedEditor.addAction({
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
				diffEditorRef.current?.layout();
			});
			resizeObserver.observe(container);
			resizeObserverRef.current = resizeObserver;
		};

		initDiffEditor().catch((error) => {
			console.error("Failed to initialize Monaco DiffEditor:", error);
		});

		return () => {
			isMounted = false;
			resizeObserverRef.current?.disconnect();
			contentChangeListenerRef.current?.dispose();
			diffEditorRef.current?.dispose();
			originalModelRef.current?.dispose();
			modifiedModelRef.current?.dispose();
		};
	}, [containerRef, language, renderSideBySide, filePath]);

	useEffect(() => {
		const diffEditor = diffEditorRef.current;
		if (!diffEditor || fontSize == null) return;
		diffEditor.updateOptions({ fontSize });
	}, [fontSize]);

	useEffect(() => {
		const originalModel = originalModelRef.current;
		if (!originalModel) return;

		if (originalModel.getValue() !== originalValue) {
			originalModel.setValue(originalValue);
		}
	}, [originalValue]);

	useEffect(() => {
		const modifiedModel = modifiedModelRef.current;
		const diffEditor = diffEditorRef.current;
		if (!modifiedModel || !diffEditor) return;

		if (modifiedModel.getValue() !== modifiedValue) {
			const modifiedEditor = diffEditor.getModifiedEditor();
			const scrollTop = modifiedEditor.getScrollTop();
			const position = modifiedEditor.getPosition();

			modifiedModel.setValue(modifiedValue);

			modifiedEditor.setScrollTop(scrollTop);
			if (position) {
				modifiedEditor.setPosition(position);
			}
		}
	}, [modifiedValue]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady triggers rebuild when editor becomes available
	useEffect(() => {
		const diffEditor = diffEditorRef.current;
		if (!diffEditor || !changeGroups) return;

		const modifiedEditor = diffEditor.getModifiedEditor();
		const editorDomNode = modifiedEditor.getDomNode();
		if (!editorDomNode) return;

		if (hunkOverlayContainerRef.current) {
			hunkOverlayContainerRef.current.remove();
		}

		const overlayContainer = document.createElement("div");
		overlayContainer.className = "hunk-overlay-container";
		editorDomNode.appendChild(overlayContainer);
		hunkOverlayContainerRef.current = overlayContainer;

		let overlays: HunkOverlay[] = [];

		const rebuild = () => {
			for (const o of overlays) o.domNode.remove();
			const lineChanges = diffEditor.getLineChanges() ?? [];
			overlays = createHunkOverlaysFromLineChanges(
				overlayContainer,
				lineChanges,
				changeGroups,
				onStageHunkRef,
				onUnstageHunkRef,
			);
			updateOverlayPositions(modifiedEditor, overlays);
		};

		const update = () => updateOverlayPositions(modifiedEditor, overlays);
		rebuild();
		const d1 = modifiedEditor.onDidScrollChange(update);
		const d2 = modifiedEditor.onDidContentSizeChange(update);
		const d3 = diffEditor.onDidUpdateDiff(rebuild);

		return () => {
			d1.dispose();
			d2.dispose();
			d3.dispose();
			overlayContainer.remove();
			hunkOverlayContainerRef.current = null;
		};
	}, [changeGroups, editorReady]);

	useEffect(() => {
		const diffEditor = diffEditorRef.current;
		const monaco = monacoRef.current;
		if (!diffEditor || !monaco || !commentRanges) return;

		const modifiedEditor = diffEditor.getModifiedEditor();
		const decorations: Monaco.editor.IModelDeltaDecoration[] =
			commentRanges.map((r) => ({
				range: new monaco.Range(r.start, 1, r.end ?? r.start, 1),
				options: {
					isWholeLine: true,
					glyphMarginClassName: "comment-marker",
				},
			}));

		commentDecorationsRef.current = modifiedEditor.deltaDecorations(
			commentDecorationsRef.current,
			decorations,
		);
	}, [commentRanges]);

	useEffect(() => {
		const diffEditor = diffEditorRef.current;
		if (!diffEditor || !revealLine) return;
		const modifiedEditor = diffEditor.getModifiedEditor();
		modifiedEditor.revealLineInCenter(revealLine.line);
		modifiedEditor.setPosition({ lineNumber: revealLine.line, column: 1 });
	}, [revealLine]);

	useEffect(() => {
		const monaco = monacoRef.current;
		if (!monaco || !theme) return;
		monaco.editor.setTheme(getMonacoThemeName(theme));
	}, [theme]);

	return {
		diffEditorRef,
		monacoRef,
	};
}
