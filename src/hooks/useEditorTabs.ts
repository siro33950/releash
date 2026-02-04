import { readTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useRef, useState } from "react";
import type { TabInfo } from "@/types/editor";

export interface UseEditorTabsReturn {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	openFile: (path: string) => Promise<void>;
	closeTab: (path: string) => void;
	setActiveTab: (path: string) => void;
	reloadTabIfClean: (path: string) => Promise<void>;
}

function getLanguageFromPath(path: string): string {
	const ext = path.split(".").pop()?.toLowerCase() ?? "";
	const languageMap: Record<string, string> = {
		ts: "typescript",
		tsx: "typescript",
		js: "javascript",
		jsx: "javascript",
		json: "json",
		md: "markdown",
		css: "css",
		scss: "scss",
		less: "less",
		html: "html",
		xml: "xml",
		yaml: "yaml",
		yml: "yaml",
		toml: "toml",
		rs: "rust",
		go: "go",
		py: "python",
		rb: "ruby",
		java: "java",
		kt: "kotlin",
		swift: "swift",
		c: "c",
		cpp: "cpp",
		h: "c",
		hpp: "cpp",
		cs: "csharp",
		sh: "shell",
		bash: "shell",
		zsh: "shell",
		sql: "sql",
		graphql: "graphql",
		vue: "vue",
		svelte: "svelte",
	};
	return languageMap[ext] ?? "plaintext";
}

function getFileNameFromPath(path: string): string {
	return path.split(/[/\\]/).pop() ?? path;
}

export function useEditorTabs(): UseEditorTabsReturn {
	const [tabs, setTabs] = useState<TabInfo[]>([]);
	const [activeTabPath, setActiveTabPath] = useState<string | null>(null);
	const tabsRef = useRef<TabInfo[]>([]);
	const pendingOpenRef = useRef<Set<string>>(new Set());
	tabsRef.current = tabs;

	const activeTab = tabs.find((tab) => tab.path === activeTabPath) ?? null;

	const openFile = useCallback(async (path: string) => {
		if (pendingOpenRef.current.has(path)) {
			setActiveTabPath(path);
			return;
		}

		const existingTab = tabsRef.current.find((tab) => tab.path === path);
		if (existingTab) {
			setActiveTabPath(path);
			return;
		}

		pendingOpenRef.current.add(path);
		try {
			const content = await readTextFile(path);
			if (tabsRef.current.some((tab) => tab.path === path)) {
				setActiveTabPath(path);
				return;
			}
			const newTab: TabInfo = {
				path,
				name: getFileNameFromPath(path),
				content,
				originalContent: content,
				isDirty: false,
				language: getLanguageFromPath(path),
			};

			setTabs((prevTabs) => [...prevTabs, newTab]);
			setActiveTabPath(path);
		} catch (error) {
			console.error(`Failed to open file: ${path}`, error);
		} finally {
			pendingOpenRef.current.delete(path);
		}
	}, []);

	const closeTab = useCallback((path: string) => {
		setTabs((prevTabs) => {
			const newTabs = prevTabs.filter((tab) => tab.path !== path);

			setActiveTabPath((currentPath) => {
				if (currentPath !== path) {
					return currentPath;
				}
				if (newTabs.length === 0) {
					return null;
				}
				const closedIndex = prevTabs.findIndex((tab) => tab.path === path);
				if (closedIndex === -1) {
					return newTabs[0].path;
				}
				const newIndex = Math.min(closedIndex, newTabs.length - 1);
				return newTabs[newIndex].path;
			});

			return newTabs;
		});
	}, []);

	const setActiveTab = useCallback((path: string) => {
		setActiveTabPath(path);
	}, []);

	const reloadTabIfClean = useCallback(async (path: string) => {
		const existingTab = tabsRef.current.find((tab) => tab.path === path);
		if (!existingTab || existingTab.isDirty) {
			return;
		}

		try {
			const content = await readTextFile(path);
			setTabs((prevTabs) =>
				prevTabs.map((tab) =>
					tab.path === path
						? { ...tab, content, originalContent: content }
						: tab,
				),
			);
		} catch (error) {
			console.error(`Failed to reload file: ${path}`, error);
		}
	}, []);

	return {
		tabs,
		activeTab,
		openFile,
		closeTab,
		setActiveTab,
		reloadTabIfClean,
	};
}
