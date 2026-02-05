import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useRef, useState } from "react";
import type { TabInfo } from "@/types/editor";

export interface UseEditorTabsReturn {
	tabs: TabInfo[];
	activeTab: TabInfo | null;
	openFile: (path: string) => Promise<void>;
	closeTab: (path: string) => void;
	setActiveTab: (path: string) => void;
	reloadTabIfClean: (path: string) => Promise<void>;
	updateTabContent: (path: string, content: string) => void;
	saveFile: (path: string) => Promise<void>;
	updateTabPath: (oldPath: string, newPath: string) => void;
	closeTabsByPrefix: (pathPrefix: string) => void;
	closeAllTabs: () => void;
	saveAllDirtyTabs: () => Promise<void>;
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

	const updateTabContent = useCallback((path: string, content: string) => {
		setTabs((prevTabs) =>
			prevTabs.map((tab) =>
				tab.path === path
					? { ...tab, content, isDirty: content !== tab.originalContent }
					: tab,
			),
		);
	}, []);

	const saveFile = useCallback(async (path: string) => {
		const tab = tabsRef.current.find((t) => t.path === path);
		if (!tab) return;

		try {
			await writeTextFile(path, tab.content);
			setTabs((prevTabs) =>
				prevTabs.map((t) =>
					t.path === path
						? { ...t, originalContent: t.content, isDirty: false }
						: t,
				),
			);
		} catch (error) {
			console.error(`Failed to save file: ${path}`, error);
		}
	}, []);

	const updateTabPath = useCallback((oldPath: string, newPath: string) => {
		setTabs((prevTabs) =>
			prevTabs.map((tab) =>
				tab.path === oldPath
					? {
							...tab,
							path: newPath,
							name: getFileNameFromPath(newPath),
							language: getLanguageFromPath(newPath),
						}
					: tab,
			),
		);
		setActiveTabPath((current) => (current === oldPath ? newPath : current));
	}, []);

	const closeTabsByPrefix = useCallback((pathPrefix: string) => {
		setTabs((prevTabs) => {
			const newTabs = prevTabs.filter(
				(tab) =>
					tab.path !== pathPrefix && !tab.path.startsWith(`${pathPrefix}/`),
			);

			setActiveTabPath((currentPath) => {
				if (!currentPath) return null;
				const isRemoved =
					currentPath === pathPrefix ||
					currentPath.startsWith(`${pathPrefix}/`);
				if (!isRemoved) return currentPath;
				if (newTabs.length === 0) return null;
				return newTabs[0].path;
			});

			return newTabs;
		});
	}, []);

	const closeAllTabs = useCallback(() => {
		setTabs([]);
		setActiveTabPath(null);
	}, []);

	const saveAllDirtyTabs = useCallback(async () => {
		const dirtyTabs = tabsRef.current.filter((t) => t.isDirty);
		await Promise.all(dirtyTabs.map((t) => saveFile(t.path)));
	}, [saveFile]);

	const reloadTabIfClean = useCallback(async (path: string) => {
		const existingTab = tabsRef.current.find((tab) => tab.path === path);
		if (!existingTab || existingTab.isDirty) {
			return;
		}

		try {
			const content = await readTextFile(path);
			setTabs((prevTabs) =>
				prevTabs.map((tab) =>
					tab.path === path && !tab.isDirty
						? { ...tab, content, originalContent: content, isDirty: false }
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
		updateTabContent,
		saveFile,
		updateTabPath,
		closeTabsByPrefix,
		closeAllTabs,
		saveAllDirtyTabs,
	};
}
