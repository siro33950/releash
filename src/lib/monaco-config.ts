import type * as Monaco from "monaco-editor";
import type { Theme } from "@/types/settings";

export const MONACO_DARK_THEME_NAME = "releash-dark";
export const MONACO_LIGHT_THEME_NAME = "releash-light";

export const MONACO_THEME_NAME = MONACO_DARK_THEME_NAME;

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
		"widget.shadow": "#00000040",
		"diffEditor.insertedTextBackground": "#9ccc2c33",
		"diffEditor.removedTextBackground": "#ff000033",
	},
};

export const DIFF_ADDED_COLOR = "#9ccc2c";
export const DIFF_MODIFIED_COLOR = "#9ccc2c";
export const DIFF_DELETED_COLOR = "#ff0000";

export const monacoLightTheme: Monaco.editor.IStandaloneThemeData = {
	base: "vs",
	inherit: true,
	rules: [
		{ token: "", foreground: "1a1a1a", background: "f8f8f8" },
		{ token: "comment", foreground: "6a737d" },
		{ token: "keyword", foreground: "d73a49" },
		{ token: "string", foreground: "22863a" },
		{ token: "number", foreground: "e36209" },
		{ token: "type", foreground: "005cc5" },
	],
	colors: {
		"editor.background": "#f8f8f8",
		"editor.foreground": "#1a1a1a",
		"editorCursor.foreground": "#1a1a1a",
		"editor.lineHighlightBackground": "#f0f0f0",
		"editorLineNumber.foreground": "#999999",
		"editorLineNumber.activeForeground": "#1a1a1a",
		"editor.selectionBackground": "#c8e1ff",
		"editor.inactiveSelectionBackground": "#e0e0e0",
		"widget.shadow": "#00000015",
		"diffEditor.insertedTextBackground": "#9ccc2c40",
		"diffEditor.removedTextBackground": "#ff000033",
	},
};

export function getMonacoThemeName(theme: Theme): string {
	return theme === "light" ? MONACO_LIGHT_THEME_NAME : MONACO_DARK_THEME_NAME;
}

interface DiagnosticsDefaults {
	setDiagnosticsOptions(options: {
		noSemanticValidation: boolean;
		noSyntaxValidation: boolean;
		noSuggestionDiagnostics: boolean;
	}): void;
}

interface TypescriptModule {
	typescriptDefaults: DiagnosticsDefaults;
	javascriptDefaults: DiagnosticsDefaults;
}

let diagnosticsDisabled = false;
export function disableBuiltinDiagnostics(monaco: typeof Monaco) {
	if (diagnosticsDisabled) return;
	diagnosticsDisabled = true;
	const ts = monaco.languages.typescript as unknown as TypescriptModule;
	const diagnosticsOptions = {
		noSemanticValidation: true,
		noSyntaxValidation: false,
		noSuggestionDiagnostics: true,
	};
	ts.typescriptDefaults.setDiagnosticsOptions(diagnosticsOptions);
	ts.javascriptDefaults.setDiagnosticsOptions(diagnosticsOptions);
}

export const defaultEditorOptions: Monaco.editor.IStandaloneEditorConstructionOptions =
	{
		contextmenu: false,
		fontSize: 14,
		fontFamily: 'Menlo, Monaco, "Courier New", monospace',
		automaticLayout: false,
		minimap: { enabled: false },
		wordWrap: "on",
		scrollBeyondLastLine: false,
		renderWhitespace: "selection",
		tabSize: 2,
		insertSpaces: true,
		cursorBlinking: "blink",
		cursorStyle: "line",
		lineNumbers: "on",
		folding: true,
		glyphMargin: false,
		lineDecorationsWidth: 0,
		lineNumbersMinChars: 4,
	};

export const defaultDiffEditorOptions: Monaco.editor.IDiffEditorConstructionOptions =
	{
		contextmenu: false,
		fontSize: 14,
		fontFamily: 'Menlo, Monaco, "Courier New", monospace',
		automaticLayout: false,
		readOnly: false,
		originalEditable: false,
		diffAlgorithm: "advanced",
		renderSideBySide: true,
		enableSplitViewResizing: true,
		renderOverviewRuler: false,
		minimap: { enabled: false },
		ignoreTrimWhitespace: false,
		renderMarginRevertIcon: false,
		renderGutterMenu: false,
		glyphMargin: true,
		useInlineViewWhenSpaceIsLimited: false,
	};
