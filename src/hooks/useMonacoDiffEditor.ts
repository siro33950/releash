import { loader } from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef } from "react";
import {
	defaultDiffEditorOptions,
	MONACO_THEME_NAME,
	monacoTheme,
} from "@/lib/monaco-config";

interface UseMonacoDiffEditorOptions {
	originalValue: string;
	modifiedValue: string;
	language?: string;
	renderSideBySide?: boolean;
	onContentChange?: (content: string) => void;
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
	} = options;

	const diffEditorRef = useRef<Monaco.editor.IStandaloneDiffEditor | null>(
		null,
	);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const originalModelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const modifiedModelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const contentChangeListenerRef = useRef<Monaco.IDisposable | null>(null);
	const originalValueRef = useRef(originalValue);
	const modifiedValueRef = useRef(modifiedValue);
	const onContentChangeRef = useRef(onContentChange);
	originalValueRef.current = originalValue;
	modifiedValueRef.current = modifiedValue;
	onContentChangeRef.current = onContentChange;

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const initDiffEditor = async () => {
			const monaco = await loader.init();

			if (!isMounted) return;

			monacoRef.current = monaco;

			monaco.editor.defineTheme(MONACO_THEME_NAME, monacoTheme);
			monaco.editor.setTheme(MONACO_THEME_NAME);

			const originalModel = monaco.editor.createModel(
				originalValueRef.current,
				language,
			);
			const modifiedModel = monaco.editor.createModel(
				modifiedValueRef.current,
				language,
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
				theme: MONACO_THEME_NAME,
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
	}, [containerRef, language, renderSideBySide]);

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

	return {
		diffEditorRef,
		monacoRef,
	};
}
