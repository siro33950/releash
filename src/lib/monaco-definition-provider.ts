import { invoke } from "@tauri-apps/api/core";
import { readTextFile } from "@tauri-apps/plugin-fs";
import type * as Monaco from "monaco-editor";
import { normalizePath } from "@/lib/normalizePath";
import type { DefinitionLocation, SearchMatch } from "@/types/search";

const SUPPORTED_LANGUAGES = [
	"typescript",
	"javascript",
	"typescriptreact",
	"javascriptreact",
	"rust",
	"python",
	"go",
];

let providersRegistered = false;

export interface NavigationCallbacks {
	onOpenFileAtLine: (relativePath: string, line: number) => void;
	getRootPath: () => string | null;
}

function getLanguageForProvider(language: string): string {
	switch (language) {
		case "typescriptreact":
			return "typescript";
		case "javascriptreact":
			return "javascript";
		default:
			return language;
	}
}

async function ensureModelsForFiles(
	monaco: typeof Monaco,
	rootPath: string,
	relativePaths: string[],
): Promise<void> {
	const uniquePaths = [...new Set(relativePaths)];
	const promises = uniquePaths.map(async (relPath) => {
		const uri = monaco.Uri.file(`${rootPath}/${relPath}`);
		if (monaco.editor.getModel(uri)) return;
		try {
			const content = await readTextFile(`${rootPath}/${relPath}`);
			if (!monaco.editor.getModel(uri)) {
				monaco.editor.createModel(content, undefined, uri);
			}
		} catch {
			// file unreadable, skip
		}
	});
	await Promise.all(promises);
}

export function registerDefinitionProviders(
	monaco: typeof Monaco,
	callbacks: NavigationCallbacks,
) {
	if (providersRegistered) return;
	providersRegistered = true;

	for (const lang of SUPPORTED_LANGUAGES) {
		const searchLang = getLanguageForProvider(lang);

		monaco.languages.registerDefinitionProvider(lang, {
			provideDefinition: async (
				model: Monaco.editor.ITextModel,
				position: Monaco.Position,
			): Promise<Monaco.languages.Location[] | null> => {
				const rootPath = callbacks.getRootPath();
				if (!rootPath) return null;

				const word = model.getWordAtPosition(position);
				if (!word) return null;

				try {
					const results = await invoke<DefinitionLocation[]>(
						"find_definition",
						{
							rootPath,
							symbol: word.word,
							language: searchLang,
						},
					);

					if (results.length === 0) return null;

					await ensureModelsForFiles(
						monaco,
						rootPath,
						results.map((r) => r.path),
					);

					return results.map((r) => ({
						uri: monaco.Uri.file(`${rootPath}/${r.path}`),
						range: new monaco.Range(
							r.line_number,
							r.column,
							r.line_number,
							r.column,
						),
					}));
				} catch {
					return null;
				}
			},
		});

		monaco.languages.registerReferenceProvider(lang, {
			provideReferences: async (
				model: Monaco.editor.ITextModel,
				position: Monaco.Position,
			): Promise<Monaco.languages.Location[] | null> => {
				const rootPath = callbacks.getRootPath();
				if (!rootPath) return null;

				const word = model.getWordAtPosition(position);
				if (!word) return null;

				try {
					const results = await invoke<SearchMatch[]>("find_references", {
						rootPath,
						symbol: word.word,
					});

					if (results.length === 0) return null;

					await ensureModelsForFiles(
						monaco,
						rootPath,
						results.map((r) => r.path),
					);

					return results.map((r) => ({
						uri: monaco.Uri.file(`${rootPath}/${r.path}`),
						range: new monaco.Range(
							r.line_number,
							r.match_start + 1,
							r.line_number,
							r.match_end + 1,
						),
					}));
				} catch {
					return null;
				}
			},
		});
	}

	monaco.editor.registerEditorOpener({
		openCodeEditor(
			_source: Monaco.editor.ICodeEditor,
			resource: Monaco.Uri,
			selectionOrPosition: Monaco.IRange | Monaco.IPosition | undefined,
		): boolean {
			const rootPath = callbacks.getRootPath();
			if (!rootPath) return false;

			const filePath = resource.path;
			const normalizedRoot = normalizePath(rootPath);
			const relativePath = filePath.startsWith(`${normalizedRoot}/`)
				? filePath.slice(normalizedRoot.length + 1)
				: filePath;

			const line = selectionOrPosition
				? "startLineNumber" in selectionOrPosition
					? selectionOrPosition.startLineNumber
					: selectionOrPosition.lineNumber
				: 1;

			callbacks.onOpenFileAtLine(relativePath, line);
			return true;
		},
	});
}

export function _resetForTesting() {
	providersRegistered = false;
}
