import { save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { useCallback, useRef, useState } from "react";
import { isImageFile } from "@/lib/imageUtils";
import type { TabInfo } from "@/types/editor";

export interface UseFileContentsReturn {
	files: TabInfo[];
	getFileContent: (path: string) => TabInfo | undefined;
	openFile: (path: string) => Promise<void>;
	closeFile: (path: string) => void;
	updateContent: (path: string, content: string) => void;
	saveFile: (path: string) => Promise<void>;
	reloadFileIfClean: (path: string) => Promise<void>;
	updateFilePath: (oldPath: string, newPath: string) => void;
	closeFilesByPrefix: (pathPrefix: string) => void;
	closeAllFiles: () => void;
	saveAllDirtyFiles: () => Promise<void>;
	createUntitledFile: () => string;
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

export function detectEol(content: string): "LF" | "CRLF" {
	return content.includes("\r\n") ? "CRLF" : "LF";
}

function getFileNameFromPath(path: string): string {
	return path.split(/[/\\]/).pop() ?? path;
}

export function useFileContents(): UseFileContentsReturn {
	const [files, setFiles] = useState<TabInfo[]>([]);
	const filesRef = useRef<TabInfo[]>([]);
	const pendingOpenRef = useRef<Set<string>>(new Set());
	const untitledCounterRef = useRef(0);
	filesRef.current = files;

	const getFileContent = useCallback(
		(path: string) => filesRef.current.find((f) => f.path === path),
		[],
	);

	const openFile = useCallback(async (path: string) => {
		if (pendingOpenRef.current.has(path)) return;

		const existing = filesRef.current.find((f) => f.path === path);
		if (existing) return;

		pendingOpenRef.current.add(path);
		try {
			const content = isImageFile(path) ? "" : await readTextFile(path);
			if (filesRef.current.some((f) => f.path === path)) return;
			const newFile: TabInfo = {
				path,
				name: getFileNameFromPath(path),
				content,
				originalContent: content,
				isDirty: false,
				language: getLanguageFromPath(path),
				eol: detectEol(content),
			};
			// filesRefを即座に更新し、model.doAction後のfactory呼び出しで
			// ファイルコンテンツが確実に取得できるようにする
			// (SizeTrackerのReact.memoにより、後からの再レンダリングが保証されないため)
			filesRef.current = [...filesRef.current, newFile];
			setFiles((prev) => [...prev, newFile]);
		} catch (error) {
			console.error(`Failed to open file: ${path}`, error);
		} finally {
			pendingOpenRef.current.delete(path);
		}
	}, []);

	const closeFile = useCallback((path: string) => {
		setFiles((prev) => prev.filter((f) => f.path !== path));
	}, []);

	const updateContent = useCallback((path: string, content: string) => {
		setFiles((prev) =>
			prev.map((f) =>
				f.path === path
					? { ...f, content, isDirty: content !== f.originalContent }
					: f,
			),
		);
	}, []);

	const saveFile = useCallback(async (path: string) => {
		const file = filesRef.current.find((f) => f.path === path);
		if (!file) return;
		if (isImageFile(path)) return;

		try {
			if (file.isUntitled) {
				const savePath = await save({
					title: "Save File",
					defaultPath: file.name,
				});
				if (!savePath) return;

				await writeTextFile(savePath, file.content);
				setFiles((prev) => {
					const withoutDuplicate = prev.filter(
						(f) => f.path !== savePath || f.path === path,
					);
					return withoutDuplicate.map((f) =>
						f.path === path
							? {
									...f,
									path: savePath,
									name: getFileNameFromPath(savePath),
									language: getLanguageFromPath(savePath),
									originalContent: f.content,
									isDirty: false,
									isUntitled: false,
								}
							: f,
					);
				});
			} else {
				await writeTextFile(path, file.content);
				setFiles((prev) =>
					prev.map((f) =>
						f.path === path
							? { ...f, originalContent: f.content, isDirty: false }
							: f,
					),
				);
			}
		} catch (error) {
			console.error(`Failed to save file: ${path}`, error);
		}
	}, []);

	const reloadFileIfClean = useCallback(async (path: string) => {
		const existing = filesRef.current.find((f) => f.path === path);
		if (!existing || existing.isDirty || isImageFile(path)) return;

		try {
			const content = await readTextFile(path);
			setFiles((prev) =>
				prev.map((f) =>
					f.path === path && !f.isDirty
						? {
								...f,
								content,
								originalContent: content,
								isDirty: false,
								eol: detectEol(content),
							}
						: f,
				),
			);
		} catch (error) {
			console.error(`Failed to reload file: ${path}`, error);
		}
	}, []);

	const updateFilePath = useCallback((oldPath: string, newPath: string) => {
		setFiles((prev) =>
			prev.map((f) =>
				f.path === oldPath
					? {
							...f,
							path: newPath,
							name: getFileNameFromPath(newPath),
							language: getLanguageFromPath(newPath),
						}
					: f,
			),
		);
	}, []);

	const closeFilesByPrefix = useCallback((pathPrefix: string) => {
		setFiles((prev) =>
			prev.filter(
				(f) => f.path !== pathPrefix && !f.path.startsWith(`${pathPrefix}/`),
			),
		);
	}, []);

	const closeAllFiles = useCallback(() => {
		setFiles([]);
	}, []);

	const saveAllDirtyFiles = useCallback(async () => {
		const dirty = filesRef.current.filter((f) => f.isDirty);
		await Promise.all(dirty.map((f) => saveFile(f.path)));
	}, [saveFile]);

	const createUntitledFile = useCallback(() => {
		untitledCounterRef.current += 1;
		const count = untitledCounterRef.current;
		const name = `Untitled-${count}`;
		const path = `untitled:${name}`;
		const newFile: TabInfo = {
			path,
			name,
			content: "",
			originalContent: "",
			isDirty: true,
			language: "plaintext",
			eol: "LF",
			isUntitled: true,
		};
		setFiles((prev) => [...prev, newFile]);
		return path;
	}, []);

	return {
		files,
		getFileContent,
		openFile,
		closeFile,
		updateContent,
		saveFile,
		reloadFileIfClean,
		updateFilePath,
		closeFilesByPrefix,
		closeAllFiles,
		saveAllDirtyFiles,
		createUntitledFile,
	};
}
