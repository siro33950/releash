import type * as Monaco from "monaco-editor";

export const MONACO_THEME_NAME = "releash-dark";

export const monacoTheme: Monaco.editor.IStandaloneThemeData = {
	base: "vs-dark",
	inherit: true,
	rules: [
		{ token: "", foreground: "e0e0e0", background: "1a1a1a" },
		{ token: "comment", foreground: "7f7f7f" },
		{ token: "keyword", foreground: "d75fff" },
		{ token: "string", foreground: "27c93f" },
		{ token: "number", foreground: "ffbd2e" },
		{ token: "type", foreground: "2ea6ff" },
	],
	colors: {
		"editor.background": "#1a1a1a",
		"editor.foreground": "#e0e0e0",
		"editorCursor.foreground": "#e0e0e0",
		"editor.lineHighlightBackground": "#2a2a2a",
		"editorLineNumber.foreground": "#7f7f7f",
		"editorLineNumber.activeForeground": "#e0e0e0",
		"editor.selectionBackground": "#3a3a3a",
		"editor.inactiveSelectionBackground": "#2a2a2a",
	},
};

export const defaultEditorOptions: Monaco.editor.IStandaloneEditorConstructionOptions =
	{
		fontSize: 14,
		fontFamily: 'Menlo, Monaco, "Courier New", monospace',
		automaticLayout: false,
		minimap: { enabled: true },
		wordWrap: "on",
		scrollBeyondLastLine: false,
		renderWhitespace: "selection",
		tabSize: 2,
		insertSpaces: false,
		cursorBlinking: "blink",
		cursorStyle: "line",
		lineNumbers: "on",
		folding: true,
		glyphMargin: false,
		lineDecorationsWidth: 0,
		lineNumbersMinChars: 4,
	};
