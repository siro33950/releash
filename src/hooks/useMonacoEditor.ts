import { loader } from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef } from "react";
import {
	defaultEditorOptions,
	MONACO_THEME_NAME,
	monacoTheme,
} from "@/lib/monaco-config";

interface UseMonacoEditorOptions {
	defaultValue?: string;
	language?: string;
	onChange?: (value: string | undefined) => void;
}

export function useMonacoEditor(
	containerRef: RefObject<HTMLDivElement | null>,
	options: UseMonacoEditorOptions = {},
) {
	const { defaultValue = "", language = "typescript", onChange } = options;
	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const onChangeRef = useRef(onChange);

	// onChangeの参照を常に最新に保つ
	useEffect(() => {
		onChangeRef.current = onChange;
	}, [onChange]);

	useEffect(() => {
		const container = containerRef.current;
		if (!container) return;

		let isMounted = true;

		const initEditor = async () => {
			const monaco = await loader.init();

			if (!isMounted) return;

			monacoRef.current = monaco;

			monaco.editor.defineTheme(MONACO_THEME_NAME, monacoTheme);
			monaco.editor.setTheme(MONACO_THEME_NAME);

			const editor = monaco.editor.create(container, {
				...defaultEditorOptions,
				value: defaultValue,
				language,
				theme: MONACO_THEME_NAME,
			});

			if (!isMounted) {
				editor.dispose();
				return;
			}

			editorRef.current = editor;

			editor.onDidChangeModelContent(() => {
				onChangeRef.current?.(editor.getValue());
			});

			const resizeObserver = new ResizeObserver(() => {
				editorRef.current?.layout();
			});
			resizeObserver.observe(container);
			resizeObserverRef.current = resizeObserver;
		};

		initEditor().catch((error) => {
			console.error("Failed to initialize Monaco Editor:", error);
		});

		return () => {
			isMounted = false;
			resizeObserverRef.current?.disconnect();
			editorRef.current?.dispose();
		};
	}, [containerRef, defaultValue, language]);

	return {
		editorRef,
		monacoRef,
	};
}
