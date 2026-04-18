import { loader } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import type * as Monaco from "monaco-editor";
import { useEffect, useRef, useState } from "react";
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

export interface CodeDiffViewerProps {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
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
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: {
	originalContent: string;
	modifiedContent: string;
	language: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const decorationsRef = useRef<string[]>([]);
	const overlayRef = useRef<HTMLDivElement>(null);

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
		});

		return () => {
			disposed = true;
			const editor = editorRef.current;
			if (editor) {
				editor.getModel()?.dispose();
				editor.dispose();
				editorRef.current = null;
			}
		};
	}, []);

	// Update content + decorations
	useEffect(() => {
		const editor = editorRef.current;
		const monaco = monacoRef.current;
		if (!editor || !monaco) return;

		const model = editor.getModel();
		if (model && model.getValue() !== modifiedContent) {
			model.setValue(modifiedContent);
		}

		const diff = computeDiff(originalContent, modifiedContent);
		const newDecorations = createDiffDecorations(diff, monaco);
		decorationsRef.current = editor.deltaDecorations(
			decorationsRef.current,
			newDecorations,
		);
	}, [originalContent, modifiedContent]);

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

	return (
		<div className="h-full w-full relative" data-testid="code-diff-viewer">
			<div ref={containerRef} className="h-full w-full" />
			<div ref={overlayRef} className="hunk-overlay-container" />
		</div>
	);
}

/**
 * Inline / Split mode: Monaco DiffEditor with inline or side-by-side rendering.
 */
function MonacoDiffViewer({
	originalContent,
	modifiedContent,
	diffMode,
	language,
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: {
	originalContent: string;
	modifiedContent: string;
	diffMode: DiffMode;
	language: string;
	changeGroups?: ChangeGroup[];
	onStageGroup?: (groupIndex: number) => void;
	groupActionLabel?: string;
}) {
	const containerRef = useRef<HTMLDivElement>(null);
	const editorRef = useRef<Monaco.editor.IDiffEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const overlayRef = useRef<HTMLDivElement>(null);

	// Always-latest refs so the async init callback sees current values
	const latestOriginalRef = useRef(originalContent);
	const latestModifiedRef = useRef(modifiedContent);
	const latestLanguageRef = useRef(language);
	const latestDiffModeRef = useRef(diffMode);
	const changeGroupsRef = useRef(changeGroups);
	const onStageGroupRef = useRef(onStageGroup);
	const groupActionLabelRef = useRef(groupActionLabel);
	latestOriginalRef.current = originalContent;
	latestModifiedRef.current = modifiedContent;
	latestLanguageRef.current = language;
	latestDiffModeRef.current = diffMode;
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
		};
	}, []);

	// Update content
	useEffect(() => {
		const editor = editorRef.current;
		if (!editor) return;

		const model = editor.getModel();
		if (model) {
			if (model.original.getValue() !== originalContent) {
				model.original.setValue(originalContent);
			}
			if (model.modified.getValue() !== modifiedContent) {
				model.modified.setValue(modifiedContent);
			}
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
	language,
	filePath,
	changeGroups,
	onStageGroup,
	groupActionLabel,
}: CodeDiffViewerProps) {
	const [detectedLanguage, setDetectedLanguage] = useState("plaintext");

	useEffect(() => {
		if (language || !filePath) return;
		invoke<string>("get_language_from_path", { filePath }).then(
			setDetectedLanguage,
		);
	}, [language, filePath]);

	const resolvedLanguage = language ?? detectedLanguage;

	if (diffMode === "gutter") {
		return (
			<GutterDiffViewer
				key="gutter"
				originalContent={originalContent}
				modifiedContent={modifiedContent}
				language={resolvedLanguage}
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
			language={resolvedLanguage}
			changeGroups={changeGroups}
			onStageGroup={onStageGroup}
			groupActionLabel={groupActionLabel}
		/>
	);
}
