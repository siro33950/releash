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
	} = options;

	const diffEditorRef = useRef<Monaco.editor.IStandaloneDiffEditor | null>(
		null,
	);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const originalModelRef = useRef<Monaco.editor.ITextModel | null>(null);
	const modifiedModelRef = useRef<Monaco.editor.ITextModel | null>(null);

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

			const originalModel = monaco.editor.createModel(originalValue, language);
			const modifiedModel = monaco.editor.createModel(modifiedValue, language);

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
			diffEditorRef.current?.dispose();
			originalModelRef.current?.dispose();
			modifiedModelRef.current?.dispose();
		};
	}, [containerRef, originalValue, modifiedValue, language, renderSideBySide]);

	return {
		diffEditorRef,
		monacoRef,
	};
}
