import { loader } from "@monaco-editor/react";
import { diffLines } from "diff";
import type * as Monaco from "monaco-editor";
import { type RefObject, useEffect, useRef } from "react";
import {
	defaultEditorOptions,
	MONACO_THEME_NAME,
	monacoTheme,
} from "@/lib/monaco-config";

interface UseMonacoGutterEditorOptions {
	originalValue: string;
	modifiedValue: string;
	language?: string;
}

interface DiffResult {
	added: number[];
	modified: number[];
}

function computeDiff(original: string, modified: string): DiffResult {
	const changes = diffLines(original, modified);
	const added: number[] = [];
	const modified_lines: number[] = [];

	let lineNumber = 1;

	for (const change of changes) {
		const lines = change.count || 0;

		if (change.added) {
			for (let i = 0; i < lines; i++) {
				added.push(lineNumber + i);
			}
			lineNumber += lines;
		} else if (change.removed) {
			// removed lines don't increment lineNumber in modified file
			// but we mark next line as modified if it exists
			if (lines > 0) {
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
	const { originalValue, modifiedValue, language = "typescript" } = options;

	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const decorationsRef = useRef<string[]>([]);

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
				value: modifiedValue,
				language,
				theme: MONACO_THEME_NAME,
				glyphMargin: true,
			});

			if (!isMounted) {
				editor.dispose();
				return;
			}

			editorRef.current = editor;

			const updateDecorations = () => {
				const currentValue = editor.getValue();
				const diff = computeDiff(originalValue, currentValue);
				const decorations: Monaco.editor.IModelDeltaDecoration[] = [];

				for (const line of diff.added) {
					decorations.push({
						range: new monaco.Range(line, 1, line, 1),
						options: {
							isWholeLine: true,
							glyphMarginClassName: "gutter-added",
						},
					});
				}

				for (const line of diff.modified) {
					decorations.push({
						range: new monaco.Range(line, 1, line, 1),
						options: {
							isWholeLine: true,
							glyphMarginClassName: "gutter-modified",
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
		};
	}, [containerRef, originalValue, modifiedValue, language]);

	return {
		editorRef,
		monacoRef,
	};
}
