import { loader } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import type * as Monaco from "monaco-editor";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	computeDiff,
	createDiffDecorations,
} from "@/hooks/useMonacoGutterEditor";
import type { ChangeGroup } from "@/lib/computeHunks";
import {
	defaultDiffEditorOptions,
	defaultEditorOptions,
	MONACO_DARK_THEME_NAME,
	MONACO_LIGHT_THEME_NAME,
	monacoLightTheme,
	monacoTheme,
} from "@/lib/monaco-config";
import type { DiffMode } from "@/types/settings";

interface HiddenRange {
	startLine: number;
	endLine: number;
	hiddenCount: number;
}

export interface CodeDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	language?: string;
	filePath?: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}

/**
 * Overlay floating "Stage" buttons at the top-right of each change group.
 * Uses direct DOM manipulation for performance (no React re-renders on scroll).
 */
function updateStageOverlay(
	editor: Monaco.editor.ICodeEditor,
	overlay: HTMLDivElement,
	changeGroups: ChangeGroup[] | undefined,
	onStageGroup: ((groupIndex: number) => void) | undefined,
	label = "Stage",
) {
	overlay.replaceChildren();

	if (!changeGroups?.length || !onStageGroup) return;

	const scrollTop = editor.getScrollTop();
	const editorHeight = editor.getLayoutInfo().height;

	for (const g of changeGroups) {
		const top = editor.getTopForLineNumber(g.newStart) - scrollTop;
		if (top < -30 || top > editorHeight) continue;

		const widget = document.createElement("div");
		widget.className = "hunk-widget";
		widget.style.top = `${top}px`;

		const btn = document.createElement("button");
		btn.className = "hunk-seg-btn hunk-stage";
		btn.textContent = label;
		btn.addEventListener("click", () => onStageGroup(g.groupIndex));

		widget.appendChild(btn);
		overlay.appendChild(widget);
	}
}

/**
 * Gutter mode: single Monaco editor showing the modified content
 * with gutter decorations (green/red bars) for added/deleted lines.
 */
function GutterDiffViewer({
	originalContent,
	modifiedContent,
	language,
	diffOnlyMode,
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	diffOnlyMode?: boolean;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const decorationsRef = useRef<string[]>([]);
	const overlayRef = useRef<HTMLDivElement>(null);
	const viewZoneIdsRef = useRef<string[]>([]);
	const [hiddenRanges, setHiddenRanges] = useState<HiddenRange[]>([]);
	const hiddenRangesRef = useRef<HiddenRange[]>([]);
	const [editorReady, setEditorReady] = useState(false);

	// Always-latest refs so the async init callback sees current values
	const latestOriginalRef = useRef(originalContent);
	const latestModifiedRef = useRef(modifiedContent);
	const latestLanguageRef = useRef(language);
	const changeGroupsRef = useRef(changeGroups);
	const onStageGroupRef = useRef(onStageGroup);
	const groupActionLabelRef = useRef(groupActionLabel);
	latestOriginalRef.current = originalContent;
	latestModifiedRef.current = modifiedContent;
	latestLanguageRef.current = language;
	changeGroupsRef.current = changeGroups;
	onStageGroupRef.current = onStageGroup;
	groupActionLabelRef.current = groupActionLabel;

	// Create / destroy editor
	useEffect(() => {
		let disposed = false;

		loader.init().then((monaco) => {
			if (disposed || !containerRef.current) return;
			monacoRef.current = monaco;

			monaco.editor.defineTheme(MONACO_DARK_THEME_NAME, monacoTheme);
			monaco.editor.defineTheme(MONACO_LIGHT_THEME_NAME, monacoLightTheme);

			const model = monaco.editor.createModel(
				latestModifiedRef.current,
				latestLanguageRef.current,
			);

			const editor = monaco.editor.create(containerRef.current, {
				...defaultEditorOptions,
				model,
				readOnly: true,
				glyphMargin: true,
				theme: MONACO_DARK_THEME_NAME,
			});

			// Apply initial decorations
			const diff = computeDiff(
				latestOriginalRef.current,
				latestModifiedRef.current,
			);
			const decos = createDiffDecorations(diff, monaco);
			decorationsRef.current = editor.deltaDecorations([], decos);

			// Stage overlay
			const refreshOverlay = () => {
				if (overlayRef.current) {
					updateStageOverlay(
						editor,
						overlayRef.current,
						changeGroupsRef.current,
						onStageGroupRef.current,
						groupActionLabelRef.current,
					);
				}
			};
			editor.onDidScrollChange(refreshOverlay);
			editor.onDidLayoutChange(refreshOverlay);
			requestAnimationFrame(refreshOverlay);

			editorRef.current = editor;
			setEditorReady(true);
		});

		return () => {
			disposed = true;
			const editor = editorRef.current;
			if (editor) {
				editor.getModel()?.dispose();
				editor.dispose();
				editorRef.current = null;
			}
			setEditorReady(false);
		};
	}, []);

	// Helper: apply content + decorations to editor
	const applyContentAndDecorations = useCallback(
		(
			editor: Monaco.editor.IStandaloneCodeEditor,
			monaco: typeof Monaco,
			original: string,
			modified: string,
		) => {
			const model = editor.getModel();
			if (model && model.getValue() !== modified) {
				model.setValue(modified);
			}
			const diff = computeDiff(original, modified);
			const newDecorations = createDiffDecorations(diff, monaco);
			decorationsRef.current = editor.deltaDecorations(
				decorationsRef.current,
				newDecorations,
			);
		},
		[],
	);

	// Update content + decorations (only when diffOnlyMode is OFF)
	// When diffOnlyMode is ON, updates are deferred to the combined effect below
	// to avoid showing stale decorations while hidden ranges are being recomputed.
	// biome-ignore lint/correctness/useExhaustiveDependencies: diffOnlyMode controls which branch handles content updates
	useEffect(() => {
		if (diffOnlyMode) return;
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;
		applyContentAndDecorations(
			editor,
			monaco,
			originalContent,
			modifiedContent,
		);
	}, [originalContent, modifiedContent, diffOnlyMode]);

	// Update stage overlay when changeGroups change
	useEffect(() => {
		const editor = editorRef.current;
		if (!editor || !overlayRef.current) return;
		updateStageOverlay(
			editor,
			overlayRef.current,
			changeGroups,
			onStageGroupRef.current,
			groupActionLabel,
		);
	}, [changeGroups, groupActionLabel]);

	// Update language
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const model = editor.getModel();
		if (model) {
			monaco.editor.setModelLanguage(model, language);
		}
	}, [language]);

	// Compute hidden ranges + update content/decorations atomically.
	// When diffOnlyMode is ON, content and decoration updates are batched
	// with hidden range computation to prevent the flash of full content
	// that occurs when decorations update before hidden ranges are ready.
	useEffect(() => {
		if (!diffOnlyMode) {
			hiddenRangesRef.current = [];
			setHiddenRanges([]);
			return;
		}

		let cancelled = false;

		invoke<HiddenRange[]>("compute_hidden_ranges_from_content", {
			original: originalContent,
			modified: modifiedContent,
			contextLines: 3,
		})
			.then((ranges) => {
				if (cancelled) return;
				// Apply content + decorations + hidden ranges together
				const editor = editorRef.current;
				const monaco = monacoRef.current;
				if (editor && monaco) {
					applyContentAndDecorations(
						editor,
						monaco,
						originalContent,
						modifiedContent,
					);
				}
				hiddenRangesRef.current = ranges;
				setHiddenRanges(ranges);
			})
			.catch(() => {
				if (cancelled) return;
				const editor = editorRef.current;
				const monaco = monacoRef.current;
				if (editor && monaco) {
					applyContentAndDecorations(
						editor,
						monaco,
						originalContent,
						modifiedContent,
					);
				}
				hiddenRangesRef.current = [];
				setHiddenRanges([]);
			});

		return () => {
			cancelled = true;
		};
	}, [
		diffOnlyMode,
		originalContent,
		modifiedContent,
		applyContentAndDecorations,
	]);

	// Apply/remove hidden areas + view zones
	useEffect(() => {
		if (!editorReady) return;
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		// Remove old view zones
		editor.changeViewZones((accessor) => {
			for (const id of viewZoneIdsRef.current) {
				accessor.removeZone(id);
			}
		});
		viewZoneIdsRef.current = [];

		if (!diffOnlyMode || hiddenRanges.length === 0) {
			// Clear hidden areas
			// biome-ignore lint/suspicious/noExplicitAny: setHiddenAreas is internal Monaco API
			(editor as any).setHiddenAreas?.([]);
			return;
		}

		// Set hidden areas
		const areas = hiddenRanges.map(
			(r) => new monaco.Range(r.startLine, 1, r.endLine, 1),
		);
		// biome-ignore lint/suspicious/noExplicitAny: setHiddenAreas is internal Monaco API
		(editor as any).setHiddenAreas?.(areas);

		// Add view zones (banners) before each hidden area
		const newZoneIds: string[] = [];
		editor.changeViewZones((accessor) => {
			for (const range of hiddenRanges) {
				const domNode = document.createElement("div");
				domNode.className =
					"hidden-lines-banner flex items-center justify-center text-xs text-muted-foreground cursor-pointer hover:bg-muted/50 border-y border-border";
				domNode.style.height = "22px";
				domNode.textContent = `··· ${range.hiddenCount} lines hidden ···`;
				domNode.addEventListener("click", () => {
					// Expand this specific range
					setHiddenRanges((prev) => {
						const next = prev.filter(
							(r) =>
								r.startLine !== range.startLine || r.endLine !== range.endLine,
						);
						hiddenRangesRef.current = next;
						return next;
					});
				});

				const id = accessor.addZone({
					afterLineNumber: range.startLine - 1,
					heightInPx: 22,
					domNode,
				});
				newZoneIds.push(id);
			}
		});
		viewZoneIdsRef.current = newZoneIds;
	}, [editorReady, diffOnlyMode, hiddenRanges]);

	return (
		<div className="h-full w-full relative" data-testid="code-diff-viewer">
			<div ref={containerRef} className="h-full w-full" />
			<div ref={overlayRef} className="hunk-overlay-container" />
		</div>
	);
}

function buildHideUnchangedRegionsOption(enabled: boolean) {
	return {
		enabled,
		contextLineCount: 3,
		minimumLineCount: 3,
		revealLineCount: 20,
	};
}

/**
 * Inline / Split mode: Monaco DiffEditor with inline or side-by-side rendering.
 */
function MonacoDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	diffOnlyMode,
	language,
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	diffOnlyMode?: boolean;
	language: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const editorRef = useRef<Monaco.editor.IDiffEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const overlayRef = useRef<HTMLDivElement>(null);
	const [editorReady, setEditorReady] = useState(false);

	// Always-latest refs so the async init callback sees current values
	const latestOriginalRef = useRef(originalContent);
	const latestModifiedRef = useRef(modifiedContent);
	const latestLanguageRef = useRef(language);
	const latestDiffModeRef = useRef(diffMode);
	const latestDiffOnlyModeRef = useRef(diffOnlyMode);
	const changeGroupsRef = useRef(changeGroups);
	const onStageGroupRef = useRef(onStageGroup);
	const groupActionLabelRef = useRef(groupActionLabel);
	latestOriginalRef.current = originalContent;
	latestModifiedRef.current = modifiedContent;
	latestLanguageRef.current = language;
	latestDiffModeRef.current = diffMode;
	latestDiffOnlyModeRef.current = diffOnlyMode;
	changeGroupsRef.current = changeGroups;
	onStageGroupRef.current = onStageGroup;
	groupActionLabelRef.current = groupActionLabel;

	// Create / destroy editor
	useEffect(() => {
		let disposed = false;

		loader.init().then((monaco) => {
			if (disposed || !containerRef.current) return;
			monacoRef.current = monaco;

			monaco.editor.defineTheme(MONACO_DARK_THEME_NAME, monacoTheme);
			monaco.editor.defineTheme(MONACO_LIGHT_THEME_NAME, monacoLightTheme);

			const editor = monaco.editor.createDiffEditor(containerRef.current, {
				...defaultDiffEditorOptions,
				readOnly: true,
				originalEditable: false,
				renderSideBySide: latestDiffModeRef.current === "split",
				useInlineViewWhenSpaceIsLimited: false,
				glyphMargin: true,
				theme: MONACO_DARK_THEME_NAME,
				hideUnchangedRegions: buildHideUnchangedRegionsOption(
					!!latestDiffOnlyModeRef.current,
				),
			});

			const originalModel = monaco.editor.createModel(
				latestOriginalRef.current,
				latestLanguageRef.current,
			);
			const modifiedModel = monaco.editor.createModel(
				latestModifiedRef.current,
				latestLanguageRef.current,
			);

			editor.setModel({ original: originalModel, modified: modifiedModel });

			// Stage overlay on modified editor
			const modifiedEditor = editor.getModifiedEditor();
			const refreshOverlay = () => {
				if (overlayRef.current) {
					updateStageOverlay(
						modifiedEditor,
						overlayRef.current,
						changeGroupsRef.current,
						onStageGroupRef.current,
						groupActionLabelRef.current,
					);
				}
			};
			modifiedEditor.onDidScrollChange(refreshOverlay);
			modifiedEditor.onDidLayoutChange(refreshOverlay);
			requestAnimationFrame(refreshOverlay);

			editorRef.current = editor;
			setEditorReady(true);
		});

		return () => {
			disposed = true;
			const editor = editorRef.current;
			if (editor) {
				const model = editor.getModel();
				model?.original?.dispose();
				model?.modified?.dispose();
				editor.dispose();
				editorRef.current = null;
			}
			setEditorReady(false);
		};
	}, []);

	// Update content
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const currentModel = editor.getModel();
		if (!currentModel) return;

		const origChanged = currentModel.original.getValue() !== originalContent;
		const modChanged = currentModel.modified.getValue() !== modifiedContent;

		if (!origChanged && !modChanged) return;

		if (latestDiffOnlyModeRef.current) {
			// When diffOnlyMode is on, recreate models instead of using setValue.
			// setValue causes Monaco to expand collapsed hideUnchangedRegions
			// during async diff recomputation. Recreating models forces Monaco
			// to compute a fresh diff with hideUnchangedRegions applied correctly.
			const newOriginal = monaco.editor.createModel(
				originalContent,
				latestLanguageRef.current,
			);
			const newModified = monaco.editor.createModel(
				modifiedContent,
				latestLanguageRef.current,
			);
			editor.setModel({ original: newOriginal, modified: newModified });
			currentModel.original.dispose();
			currentModel.modified.dispose();
		} else {
			if (origChanged) currentModel.original.setValue(originalContent);
			if (modChanged) currentModel.modified.setValue(modifiedContent);
		}
	}, [originalContent, modifiedContent]);

	// Update stage overlay when changeGroups change
	useEffect(() => {
		const editor = editorRef.current;
		if (!editor || !overlayRef.current) return;
		const modifiedEditor = editor.getModifiedEditor();
		updateStageOverlay(
			modifiedEditor,
			overlayRef.current,
			changeGroups,
			onStageGroupRef.current,
			groupActionLabel,
		);
	}, [changeGroups, groupActionLabel]);

	// Update language
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const model = editor.getModel();
		if (model) {
			monaco.editor.setModelLanguage(model.original, language);
			monaco.editor.setModelLanguage(model.modified, language);
		}
	}, [language]);

	// Update diff mode (side-by-side vs inline)
	useEffect(() => {
		const editor = editorRef.current;
		if (!editor) return;

		editor.updateOptions({ renderSideBySide: diffMode === "split" });
	}, [diffMode]);

	// Update hideUnchangedRegions based on diffOnlyMode
	useEffect(() => {
		if (!editorReady) return;
		const editor = editorRef.current;
		if (!editor) return;

		editor.updateOptions({
			hideUnchangedRegions: buildHideUnchangedRegionsOption(!!diffOnlyMode),
		});
	}, [editorReady, diffOnlyMode]);

	return (
		<div className="h-full w-full relative" data-testid="code-diff-viewer">
			<div ref={containerRef} className="h-full w-full" />
			<div ref={overlayRef} className="hunk-overlay-container" />
		</div>
	);
}

export function CodeDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	diffOnlyMode,
	language,
	filePath,
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: CodeDiffViewerProps) {
	const [detectedLanguage, setDetectedLanguage] = useState("plaintext");

	useEffect(() => {
		if (language || !filePath) return;
		let cancelled = false;
		invoke<string>("get_language_from_path", { filePath }).then(
			(detected) => {
				if (!cancelled) setDetectedLanguage(detected);
			},
			() => {
				if (!cancelled) setDetectedLanguage("plaintext");
			},
		);
		return () => {
			cancelled = true;
		};
	}, [language, filePath]);

	const resolvedLanguage = language ?? detectedLanguage;

	if (diffMode === "gutter") {
		return (
			<GutterDiffViewer
				key="gutter"
				originalContent={originalContent}
				modifiedContent={modifiedContent}
				language={resolvedLanguage}
				diffOnlyMode={diffOnlyMode}
				changeGroups={changeGroups}
				onStageGroup={onStageGroup}
				groupActionLabel={groupActionLabel}
			/>
		);
	}

	return (
		<MonacoDiffViewer
			key={diffMode}
			originalContent={originalContent}
			modifiedContent={modifiedContent}
			diffMode={diffMode}
			diffOnlyMode={diffOnlyMode}
			language={resolvedLanguage}
			changeGroups={changeGroups}
			onStageGroup={onStageGroup}
			groupActionLabel={groupActionLabel}
		/>
	);
}
