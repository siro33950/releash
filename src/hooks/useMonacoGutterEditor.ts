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
	onContentChange?: (content: string) => void;
}

interface DiffResult {
	added: number[];
	modified: number[];
}

export function computeDiff(original: string, modified: string): DiffResult {
	const changes = diffLines(original, modified);
	const added: number[] = [];
	const modified_lines: number[] = [];

	// modified ファイルの行数を計算（末尾改行で空要素が生成されるのを考慮）
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
			// 削除された行は modified ファイルの行番号を進めない
			// 次の行が存在する場合のみ modified としてマーク
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
	} = options;

	const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
	const monacoRef = useRef<typeof Monaco | null>(null);
	const resizeObserverRef = useRef<ResizeObserver | null>(null);
	const decorationsRef = useRef<string[]>([]);
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

		const initEditor = async () => {
			const monaco = await loader.init();

			if (!isMounted) return;

			monacoRef.current = monaco;

			monaco.editor.defineTheme(MONACO_THEME_NAME, monacoTheme);
			monaco.editor.setTheme(MONACO_THEME_NAME);

			const editor = monaco.editor.create(container, {
				...defaultEditorOptions,
				value: modifiedValueRef.current,
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
				const diff = computeDiff(originalValueRef.current, currentValue);
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
				onContentChangeRef.current?.(editor.getValue());
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
	}, [containerRef, language]);

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
	}, [originalValue]);

	return {
		editorRef,
		monacoRef,
	};
}
