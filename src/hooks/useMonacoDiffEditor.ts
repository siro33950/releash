import { loader } from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef, useState } from "react";
import {
	type CommentThreadZone,
	createCommentThread,
} from "@/lib/commentThreadWidget";
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
import type { CommentRange } from "@/types/comment";
import type { Theme } from "@/types/settings";
import type { Thread } from "@/types/thread";

interface RevealLine {
	line: number;
	key: number;
	openThread?: boolean;
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
	onAddEntry?: (threadId: string, content: string) => void;
	onDeleteThread?: (threadId: string) => void;
	onResolveThread?: (threadId: string) => void;
	onImplementThread?: (threadId: string) => void;
	onPostToPr?: (threadId: string) => void;
	onAskAI?: (threadId: string) => void;
	aiRunningThreadIds?: Set<string>;
	aiTaskThreadIds?: Set<string>;
	onOpenThreadAIModal?: (threadId?: string) => void;
	onUpdateEntry?: (threadId: string, entryId: string, content: string) => void;
	onCopyThread?: (thread: Thread) => void;
	getThreadsForLine?: (lineNumber: number) => Thread[];
	revealLine?: RevealLine;
	theme?: Theme;
	readOnly?: boolean;
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
		onAddEntry,
		onDeleteThread,
		onResolveThread,
		onImplementThread,
		onPostToPr,
		onAskAI,
		aiRunningThreadIds,
		aiTaskThreadIds,
		onOpenThreadAIModal,
		onUpdateEntry,
		onCopyThread,
		getThreadsForLine,
		revealLine,
		theme,
		readOnly,
	} = options;

	const diffEditorRef = useRef<Monaco.editor.IStandaloneDiffEditor | null>(
		null,
	);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const [editorReady, setEditorReady] = useState(false);
	const intersectionObserverRef = useRef<IntersectionObserver | null>(null);
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
	const onAddEntryRef = useRef(onAddEntry);
	const onDeleteThreadRef = useRef(onDeleteThread);
	const onResolveThreadRef = useRef(onResolveThread);
	const onImplementThreadRef = useRef(onImplementThread);
	const onPostToPrRef = useRef(onPostToPr);
	const onAskAIRef = useRef(onAskAI);
	const aiRunningThreadIdsRef = useRef(aiRunningThreadIds);
	const aiTaskThreadIdsRef = useRef(aiTaskThreadIds);
	const onOpenThreadAIModalRef = useRef(onOpenThreadAIModal);
	const onUpdateEntryRef = useRef(onUpdateEntry);
	const onCopyThreadRef = useRef(onCopyThread);
	const getThreadsForLineRef = useRef(getThreadsForLine);
	const onStageHunkRef = useRef(onStageHunk);
	const onUnstageHunkRef = useRef(onUnstageHunk);
	const hunkOverlayContainerRef = useRef<HTMLDivElement | null>(null);
	const commentInputWidgetRef = useRef<CommentThreadZone | null>(null);
	const openWidgetInfoRef = useRef<{
		threadId: string;
		lineNumber: number;
	} | null>(null);
	const dragStartLineRef = useRef<number | null>(null);
	const dragRangeDecorationsRef = useRef<string[]>([]);
	const hoverLineRef = useRef<number | null>(null);
	const hoverDecorationsRef = useRef<string[]>([]);
	const pendingRevealRef = useRef<(() => void) | null>(null);
	const isProgrammaticUpdateRef = useRef(false);
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
	onImplementThreadRef.current = onImplementThread;
	onPostToPrRef.current = onPostToPr;
	onAskAIRef.current = onAskAI;
	aiRunningThreadIdsRef.current = aiRunningThreadIds;
	aiTaskThreadIdsRef.current = aiTaskThreadIds;
	onOpenThreadAIModalRef.current = onOpenThreadAIModal;
	onUpdateEntryRef.current = onUpdateEntry;
	onCopyThreadRef.current = onCopyThread;
	getThreadsForLineRef.current = getThreadsForLine;
	onStageHunkRef.current = onStageHunk;
	onUnstageHunkRef.current = onUnstageHunk;
	themeRef.current = theme;
	commentRangesRef.current = commentRanges;

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

			const diffEditor = monaco.editor.createDiffEditor(
				container,
				{
					...defaultDiffEditorOptions,
					renderSideBySide,
					theme: themeName,
					...(fontSizeRef.current != null && {
						fontSize: fontSizeRef.current,
					}),
					...(readOnly != null && { readOnly, originalEditable: false }),
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
				diffEditor.dispose();
				originalModel.dispose();
				modifiedModel.dispose();
				return;
			}

			diffEditor.setModel({
				original: originalModel,
				modified: modifiedModel,
			});

			if (!renderSideBySide) {
				diffEditor.getOriginalEditor().updateOptions({ lineNumbers: "off" });
			}

			contentChangeListenerRef.current = diffEditor
				.getModifiedEditor()
				.onDidChangeModelContent(() => {
					if (isProgrammaticUpdateRef.current) return;
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
					onSubmit: async (content) => {
						if (isNew) {
							onAddCommentRef.current?.(lineNum, content, endLine);
							zone.dispose();
							commentInputWidgetRef.current = null;
							openWidgetInfoRef.current = null;
							ed.focus();
						} else {
							await onAddEntryRef.current?.(thread.id, content);
							onAskAIRef.current?.(thread.id);
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
					onImplementThread: (threadId) =>
						onImplementThreadRef.current?.(threadId),
					onPostToPr: (threadId) => onPostToPrRef.current?.(threadId),
					aiRunningThreadIds: aiRunningThreadIdsRef.current,
					aiTaskThreadIds: aiTaskThreadIdsRef.current,
					onOpenThreadAIModal: (tid) => onOpenThreadAIModalRef.current?.(tid),
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

			modifiedEditor.onMouseDown((e: Monaco.editor.IEditorMouseEvent) => {
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
					const hasComment =
						lineNum != null &&
						commentRangesRef.current?.some(
							(r) => lineNum >= r.start && lineNum <= (r.end ?? r.start),
						);
					hoverDecorationsRef.current = modifiedEditor.deltaDecorations(
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

			modifiedEditor.onMouseUp((e: Monaco.editor.IEditorMouseEvent) => {
				if (dragStartLineRef.current == null) return;

				const startLine = dragStartLineRef.current;
				dragStartLineRef.current = null;

				dragRangeDecorationsRef.current = modifiedEditor.deltaDecorations(
					dragRangeDecorationsRef.current,
					[],
				);

				const selection = modifiedEditor.getSelection();
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

				modifiedEditor.setSelection(new monaco.Selection(lo, 1, lo, 1));

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

			// Inactive editor tabs use visibility:hidden.
			// automaticLayout handles normal resizes, but ResizeObserver
			// may not fire on visibility transitions.
			// Use IntersectionObserver with rAF to ensure layout after
			// CSS has fully resolved.
			const intersectionObserver = new IntersectionObserver((entries) => {
				if (entries.some((e) => e.isIntersecting)) {
					requestAnimationFrame(() => {
						diffEditorRef.current?.layout();
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

		initDiffEditor().catch((error) => {
			console.error("Failed to initialize Monaco DiffEditor:", error);
		});

		return () => {
			isMounted = false;
			setEditorReady(false);
			pendingRevealRef.current = null;
			commentInputWidgetRef.current?.dispose();
			commentInputWidgetRef.current = null;
			openWidgetInfoRef.current = null;
			intersectionObserverRef.current?.disconnect();
			intersectionObserverRef.current = null;
			contentChangeListenerRef.current?.dispose();
			contentChangeListenerRef.current = null;
			diffEditorRef.current?.dispose();
			diffEditorRef.current = null;
			originalModelRef.current?.dispose();
			originalModelRef.current = null;
			modifiedModelRef.current?.dispose();
			modifiedModelRef.current = null;
		};
	}, [containerRef, language, renderSideBySide, filePath, readOnly]);

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

			isProgrammaticUpdateRef.current = true;
			try {
				modifiedModel.setValue(modifiedValue);
			} finally {
				isProgrammaticUpdateRef.current = false;
			}

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

	// biome-ignore lint/correctness/useExhaustiveDependencies: editorReady ensures editor/monaco refs are initialized before applying decorations
	useEffect(() => {
		const diffEditor = diffEditorRef.current;
		const monaco = monacoRef.current;
		if (!diffEditor || !monaco) return;

		const modifiedEditor = diffEditor.getModifiedEditor();
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

		commentDecorationsRef.current = modifiedEditor.deltaDecorations(
			commentDecorationsRef.current,
			decorations,
		);
	}, [commentRanges, editorReady]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: containerRef.current is a ref that doesn't trigger re-renders; visibility is handled by IntersectionObserver + pendingRevealRef
	useEffect(() => {
		if (!editorReady || !revealLine) return;
		const diffEditor = diffEditorRef.current;
		if (!diffEditor) return;

		const applyReveal = () => {
			const modifiedEditor = diffEditor.getModifiedEditor();
			diffEditor.layout();
			modifiedEditor.revealLineInCenter(revealLine.line);
			modifiedEditor.setPosition({ lineNumber: revealLine.line, column: 1 });

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
					const zone = createCommentThread(modifiedEditor, {
						thread,
						onSubmit: async (content) => {
							await onAddEntryRef.current?.(thread.id, content);
							onAskAIRef.current?.(thread.id);
							const textarea = zone.domNode.querySelector<HTMLTextAreaElement>(
								".comment-thread-textarea",
							);
							if (textarea) textarea.value = "";
						},
						onCancel: () => {
							zone.dispose();
							commentInputWidgetRef.current = null;
							openWidgetInfoRef.current = null;
							modifiedEditor.focus();
						},
						onDeleteThread: (threadId) => onDeleteThreadRef.current?.(threadId),
						onResolveThread: (threadId) =>
							onResolveThreadRef.current?.(threadId),
						onImplementThread: (threadId) =>
							onImplementThreadRef.current?.(threadId),
						onPostToPr: (threadId) => onPostToPrRef.current?.(threadId),
						aiRunningThreadIds: aiRunningThreadIdsRef.current,
						aiTaskThreadIds: aiTaskThreadIdsRef.current,
						onOpenThreadAIModal: (tid) => onOpenThreadAIModalRef.current?.(tid),
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
		}
	}, [getThreadsForLine]);

	// Update open widget when aiRunningThreadIds / aiTaskThreadIds changes
	useEffect(() => {
		commentInputWidgetRef.current?.update({
			aiRunningThreadIds,
			aiTaskThreadIds,
			onOpenThreadAIModal,
		});
	}, [aiRunningThreadIds, aiTaskThreadIds, onOpenThreadAIModal]);

	return {
		diffEditorRef,
		monacoRef,
	};
}
