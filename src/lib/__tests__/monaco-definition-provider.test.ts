import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	_resetForTesting,
	type NavigationCallbacks,
	registerDefinitionProviders,
} from "../monaco-definition-provider";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
	readTextFile: vi.fn(() => Promise.resolve("")),
}));

function createMockMonaco() {
	return {
		languages: {
			registerDefinitionProvider: vi.fn(),
			registerReferenceProvider: vi.fn(),
		},
		editor: {
			registerEditorOpener: vi.fn(),
			getModel: vi.fn(() => null),
			createModel: vi.fn(),
		},
		Uri: {
			file: vi.fn((path: string) => ({ path })),
		},
		Range: class {
			constructor(
				public startLineNumber: number,
				public startColumn: number,
				public endLineNumber: number,
				public endColumn: number,
			) {}
		},
	};
}

describe("registerDefinitionProviders", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		_resetForTesting();
	});

	it("should register definition providers, reference providers, and editor opener", () => {
		const monaco = createMockMonaco();
		const callbacks: NavigationCallbacks = {
			onOpenFileAtLine: vi.fn(),
			getRootPath: () => "/root",
		};

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		expect(
			monaco.languages.registerDefinitionProvider.mock.calls.length,
		).toBeGreaterThan(0);
		expect(
			monaco.languages.registerReferenceProvider.mock.calls.length,
		).toBeGreaterThan(0);
		expect(monaco.editor.registerEditorOpener).toHaveBeenCalledOnce();

		const registeredLangs =
			monaco.languages.registerDefinitionProvider.mock.calls.map(
				(c: unknown[]) => c[0],
			);
		expect(registeredLangs).toContain("typescript");
		expect(registeredLangs).toContain("javascript");
		expect(registeredLangs).toContain("rust");
		expect(registeredLangs).toContain("python");
	});

	it("should be idempotent", () => {
		const monaco = createMockMonaco();
		const callbacks: NavigationCallbacks = {
			onOpenFileAtLine: vi.fn(),
			getRootPath: () => "/root",
		};

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		const defCount =
			monaco.languages.registerDefinitionProvider.mock.calls.length;
		const openerCount = monaco.editor.registerEditorOpener.mock.calls.length;

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		expect(monaco.languages.registerDefinitionProvider.mock.calls.length).toBe(
			defCount,
		);
		expect(monaco.editor.registerEditorOpener.mock.calls.length).toBe(
			openerCount,
		);
	});

	it("editor opener should call onOpenFileAtLine with relative path and line", () => {
		const monaco = createMockMonaco();
		const onOpenFileAtLine = vi.fn();
		const callbacks: NavigationCallbacks = {
			onOpenFileAtLine,
			getRootPath: () => "/root",
		};

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		const openerArg = monaco.editor.registerEditorOpener.mock.calls[0][0];

		const result = openerArg.openCodeEditor(
			{},
			{ path: "/root/src/main.ts" },
			{ startLineNumber: 10, startColumn: 1, endLineNumber: 10, endColumn: 1 },
		);

		expect(result).toBe(true);
		expect(onOpenFileAtLine).toHaveBeenCalledWith("src/main.ts", 10);
	});

	it("editor opener should handle IPosition (lineNumber)", () => {
		const monaco = createMockMonaco();
		const onOpenFileAtLine = vi.fn();
		const callbacks: NavigationCallbacks = {
			onOpenFileAtLine,
			getRootPath: () => "/root",
		};

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		const openerArg = monaco.editor.registerEditorOpener.mock.calls[0][0];

		openerArg.openCodeEditor(
			{},
			{ path: "/root/src/lib.ts" },
			{ lineNumber: 5, column: 1 },
		);

		expect(onOpenFileAtLine).toHaveBeenCalledWith("src/lib.ts", 5);
	});

	it("editor opener should return false when rootPath is null", () => {
		const monaco = createMockMonaco();
		const callbacks: NavigationCallbacks = {
			onOpenFileAtLine: vi.fn(),
			getRootPath: () => null,
		};

		registerDefinitionProviders(
			monaco as unknown as Parameters<typeof registerDefinitionProviders>[0],
			callbacks,
		);

		const openerArg = monaco.editor.registerEditorOpener.mock.calls[0][0];

		const result = openerArg.openCodeEditor(
			{},
			{ path: "/some/file.ts" },
			{ lineNumber: 1, column: 1 },
		);

		expect(result).toBe(false);
		expect(callbacks.onOpenFileAtLine).not.toHaveBeenCalled();
	});
});
