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
	getModel: vi.fn().mockReturnValue(null),
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
		defineTheme: vi.fn(),
		setTheme: vi.fn(),
	},
	languages: {
		register: vi.fn(),
		setMonarchTokensProvider: vi.fn(),
	},
	Range: MockRange,
};

vi.mock("@monaco-editor/react", () => ({
	loader: {
		init: vi.fn().mockResolvedValue(mockMonaco),
		config: vi.fn(),
	},
}));
