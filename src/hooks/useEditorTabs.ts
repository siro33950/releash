import { readTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useRef, useState } from "react";
import type { TabInfo } from "@/types/editor";

export interface UseEditorTabsReturn {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	openFile: (path: string) => Promise<void>;
	closeTab: (path: string) => void;
	setActiveTab: (path: string) => void;
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
	return path.split("/").pop() ?? path;
}

export function useEditorTabs(): UseEditorTabsReturn {
	const [tabs, setTabs] = useState<TabInfo[]>([]);
	const [activeTabPath, setActiveTabPath] = useState<string | null>(null);
	const tabsRef = useRef<TabInfo[]>([]);
	tabsRef.current = tabs;

	const activeTab = tabs.find((tab) => tab.path === activeTabPath) ?? null;

	const openFile = useCallback(async (path: string) => {
		const existingTab = tabsRef.current.find((tab) => tab.path === path);
		if (existingTab) {
			setActiveTabPath(path);
			return;
		}

		const content = await readTextFile(path);
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
				const newIndex = Math.min(closedIndex, newTabs.length - 1);
				return newTabs[newIndex].path;
			});

			return newTabs;
		});
	}, []);

	const setActiveTab = useCallback((path: string) => {
		setActiveTabPath(path);
	}, []);

	return {
		tabs,
		activeTab,
		openFile,
		closeTab,
		setActiveTab,
	};
}
