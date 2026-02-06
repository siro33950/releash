import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

globalThis.ResizeObserver = class ResizeObserver {
	observe() {}
	unobserve() {}
	disconnect() {}
};

Object.defineProperty(window, "matchMedia", {
	writable: true,
	value: vi.fn().mockImplementation((query: string) => ({
		matches: false,
		media: query,
		onchange: null,
		addListener: vi.fn(),
		removeListener: vi.fn(),
		addEventListener: vi.fn(),
		removeEventListener: vi.fn(),
		dispatchEvent: vi.fn(),
	})),
});

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(1),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@xterm/xterm", () => {
	return {
		Terminal: class MockTerminal {
			loadAddon = vi.fn();
			open = vi.fn();
			write = vi.fn();
			onData = vi.fn().mockReturnValue({ dispose: vi.fn() });
			dispose = vi.fn();
			options: Record<string, unknown> = {};
			rows = 24;
			cols = 80;
		},
	};
});

vi.mock("@xterm/addon-fit", () => {
	return {
		FitAddon: class MockFitAddon {
			fit = vi.fn();
		},
	};
});

const mockEditor = {
	dispose: vi.fn(),
	getValue: vi.fn().mockReturnValue(""),
	setValue: vi.fn(),
	layout: vi.fn(),
	onDidChangeModelContent: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	onMouseDown: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	onMouseMove: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	onMouseUp: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	addAction: vi.fn(),
	addContentWidget: vi.fn(),
	removeContentWidget: vi.fn(),
	changeViewZones: vi
		.fn()
		.mockImplementation(
			(
				cb: (accessor: {
					addZone: () => string;
					removeZone: () => void;
				}) => void,
			) => {
				cb({ addZone: () => "zone-id", removeZone: () => {} });
			},
		),
	getDomNode: vi.fn().mockReturnValue(document.createElement("div")),
	getTopForLineNumber: vi.fn().mockReturnValue(0),
	getScrolledVisiblePosition: vi
		.fn()
		.mockReturnValue({ top: 0, left: 0, height: 20 }),
	getLayoutInfo: vi.fn().mockReturnValue({ height: 600 }),
	onDidScrollChange: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	onDidContentSizeChange: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	getModel: vi.fn().mockReturnValue(null),
	getPosition: vi.fn().mockReturnValue(null),
	getSelection: vi.fn().mockReturnValue(null),
	setPosition: vi.fn(),
	getScrollTop: vi.fn().mockReturnValue(0),
	setScrollTop: vi.fn(),
	revealLineInCenter: vi.fn(),
	focus: vi.fn(),
	updateOptions: vi.fn(),
	deltaDecorations: vi.fn().mockReturnValue([]),
};

const mockTextModel = {
	dispose: vi.fn(),
	getValue: vi.fn().mockReturnValue(""),
	setValue: vi.fn(),
	getLineCount: vi.fn().mockReturnValue(1),
	getLineContent: vi.fn().mockReturnValue(""),
};

const mockDiffEditor = {
	dispose: vi.fn(),
	layout: vi.fn(),
	getOriginalEditor: vi.fn().mockReturnValue(mockEditor),
	getModifiedEditor: vi.fn().mockReturnValue(mockEditor),
	setModel: vi.fn(),
	updateOptions: vi.fn(),
	onDidUpdateDiff: vi.fn().mockReturnValue({ dispose: vi.fn() }),
	getLineChanges: vi.fn().mockReturnValue([]),
};

class MockRange {
	startLineNumber: number;
	startColumn: number;
	endLineNumber: number;
	endColumn: number;
	constructor(
		startLineNumber: number,
		startColumn: number,
		endLineNumber: number,
		endColumn: number,
	) {
		this.startLineNumber = startLineNumber;
		this.startColumn = startColumn;
		this.endLineNumber = endLineNumber;
		this.endColumn = endColumn;
	}
}

const mockMonaco = {
	editor: {
		create: vi.fn().mockReturnValue(mockEditor),
		createDiffEditor: vi.fn().mockReturnValue(mockDiffEditor),
		createModel: vi.fn().mockReturnValue(mockTextModel),
		getModel: vi.fn().mockReturnValue(null),
		defineTheme: vi.fn(),
		setTheme: vi.fn(),
		registerEditorOpener: vi.fn(),
		MouseTargetType: { GUTTER_GLYPH_MARGIN: 2 },
		ContentWidgetPositionPreference: { ABOVE: 1 },
	},
	languages: {
		register: vi.fn(),
		setMonarchTokensProvider: vi.fn(),
		registerDefinitionProvider: vi.fn(),
		registerReferenceProvider: vi.fn(),
		typescript: {
			typescriptDefaults: {
				setDiagnosticsOptions: vi.fn(),
			},
			javascriptDefaults: {
				setDiagnosticsOptions: vi.fn(),
			},
		},
	},
	Uri: {
		file: vi.fn((path: string) => ({ scheme: "file", path })),
	},
	Range: MockRange,
	KeyMod: { CtrlCmd: 2048 },
	KeyCode: { KeyK: 41 },
};

vi.mock("@monaco-editor/react", () => ({
	loader: {
		init: vi.fn().mockResolvedValue(mockMonaco),
		config: vi.fn(),
	},
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
	readDir: vi.fn().mockResolvedValue([]),
	readTextFile: vi.fn().mockResolvedValue(""),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn().mockResolvedValue(null),
}));
