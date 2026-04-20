import type { HighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";

let highlighterPromise: Promise<HighlighterCore> | null = null;
let highlighterInstance: HighlighterCore | null = null;

async function getHighlighter(): Promise<HighlighterCore> {
	if (highlighterInstance) return highlighterInstance;
	if (highlighterPromise) return highlighterPromise;

	highlighterPromise = (async () => {
		const { createHighlighterCore } = await import("shiki/core");
		const githubDark = (await import("@shikijs/themes/github-dark")).default;

		const hl = await createHighlighterCore({
			themes: [githubDark],
			langs: [],
			engine: createJavaScriptRegexEngine(),
		});

		highlighterInstance = hl;
		return hl;
	})();

	return highlighterPromise;
}

const LANG_IMPORT_MAP = new Map<string, () => Promise<unknown>>([
	["typescript", () => import("@shikijs/langs/typescript")],
	["javascript", () => import("@shikijs/langs/javascript")],
	["rust", () => import("@shikijs/langs/rust")],
	["json", () => import("@shikijs/langs/json")],
	["toml", () => import("@shikijs/langs/toml")],
	["yaml", () => import("@shikijs/langs/yaml")],
	["html", () => import("@shikijs/langs/html")],
	["css", () => import("@shikijs/langs/css")],
	["scss", () => import("@shikijs/langs/scss")],
	["python", () => import("@shikijs/langs/python")],
	["go", () => import("@shikijs/langs/go")],
	["shell", () => import("@shikijs/langs/shellscript")],
	["sql", () => import("@shikijs/langs/sql")],
	["markdown", () => import("@shikijs/langs/markdown")],
	["xml", () => import("@shikijs/langs/xml")],
	["c", () => import("@shikijs/langs/c")],
	["cpp", () => import("@shikijs/langs/cpp")],
	["java", () => import("@shikijs/langs/java")],
	["ruby", () => import("@shikijs/langs/ruby")],
	["swift", () => import("@shikijs/langs/swift")],
	["kotlin", () => import("@shikijs/langs/kotlin")],
	["php", () => import("@shikijs/langs/php")],
	["lua", () => import("@shikijs/langs/lua")],
	["r", () => import("@shikijs/langs/r")],
	["dart", () => import("@shikijs/langs/dart")],
]);

const loadedLanguages = new Set<string>();
const loadingLanguages = new Map<string, Promise<void>>();

async function ensureLanguageLoaded(
	hl: HighlighterCore,
	language: string,
): Promise<boolean> {
	if (language === "plaintext" || loadedLanguages.has(language)) return true;

	if (hl.getLoadedLanguages().includes(language)) {
		loadedLanguages.add(language);
		return true;
	}

	const existing = loadingLanguages.get(language);
	if (existing) {
		await existing;
		return loadedLanguages.has(language);
	}

	const importFn = LANG_IMPORT_MAP.get(language);
	if (!importFn) return false;

	const promise = (async () => {
		const mod = await importFn();
		const langDef = (mod as { default: unknown }).default;
		await hl.loadLanguage(
			langDef as Parameters<HighlighterCore["loadLanguage"]>[0],
		);
		loadedLanguages.add(language);
	})();

	loadingLanguages.set(language, promise);

	try {
		await promise;
		return true;
	} catch {
		return false;
	} finally {
		loadingLanguages.delete(language);
	}
}

const THEME = "github-dark";

export interface TokenizeRequest {
	id: number;
	code: string;
	language: string;
}

export interface TokenizeResponse {
	id: number;
	lines: { tokens: { content: string; color?: string; offset: number }[] }[];
}

self.onmessage = async (e: MessageEvent<TokenizeRequest>) => {
	const { id, code, language } = e.data;

	if (code === "") {
		self.postMessage({ id, lines: [] } satisfies TokenizeResponse);
		return;
	}

	try {
		const hl = await getHighlighter();
		const loaded = await ensureLanguageLoaded(hl, language);
		const effectiveLang =
			loaded && language !== "plaintext" ? language : "text";

		const tokenLines = hl.codeToTokensBase(code, {
			lang: effectiveLang,
			theme: THEME,
		});

		const lines = tokenLines.map((tokens) => ({
			tokens: tokens.map((t) => ({
				content: t.content,
				color: t.color,
				offset: t.offset,
			})),
		}));

		self.postMessage({ id, lines } satisfies TokenizeResponse);
	} catch (err) {
		console.error("shiki worker tokenize failed:", err);
		self.postMessage({ id, lines: [] } satisfies TokenizeResponse);
	}
};
