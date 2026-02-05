import {
	copyFile,
	exists,
	mkdir,
	readDir,
	remove,
	rename,
	writeTextFile,
} from "@tauri-apps/plugin-fs";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useCallback, useState } from "react";

export interface FileClipboard {
	operation: "cut" | "copy";
	sourcePath: string;
	type: "file" | "folder";
}

export interface UseFileOperationsReturn {
	clipboard: FileClipboard | null;
	createFile: (path: string) => Promise<void>;
	createFolder: (path: string) => Promise<void>;
	deleteItem: (path: string) => Promise<void>;
	renameItem: (oldPath: string, newPath: string) => Promise<void>;
	copyPath: (path: string) => Promise<void>;
	copyRelativePath: (path: string, rootPath: string) => Promise<void>;
	revealInFinder: (path: string) => Promise<void>;
	fileExists: (path: string) => Promise<boolean>;
	cut: (path: string, type: "file" | "folder") => void;
	copy: (path: string, type: "file" | "folder") => void;
	paste: (destDir: string) => Promise<void>;
}

async function copyDirRecursive(src: string, dest: string): Promise<void> {
	await mkdir(dest, { recursive: true });
	const entries = await readDir(src);
	for (const entry of entries) {
		const srcPath = `${src}/${entry.name}`;
		const destPath = `${dest}/${entry.name}`;
		if (entry.isDirectory) {
			await copyDirRecursive(srcPath, destPath);
		} else {
			await copyFile(srcPath, destPath);
		}
	}
}

export function useFileOperations(): UseFileOperationsReturn {
	const [clipboard, setClipboard] = useState<FileClipboard | null>(null);

	const createFile = useCallback(async (path: string) => {
		await writeTextFile(path, "");
	}, []);

	const createFolder = useCallback(async (path: string) => {
		await mkdir(path, { recursive: true });
	}, []);

	const deleteItem = useCallback(async (path: string) => {
		await remove(path, { recursive: true });
	}, []);

	const renameItem = useCallback(async (oldPath: string, newPath: string) => {
		await rename(oldPath, newPath);
	}, []);

	const copyPath = useCallback(async (path: string) => {
		await navigator.clipboard.writeText(path);
	}, []);

	const copyRelativePath = useCallback(
		async (path: string, rootPath: string) => {
			const relative = path.startsWith(rootPath)
				? path.slice(rootPath.length).replace(/^\//, "")
				: path;
			await navigator.clipboard.writeText(relative);
		},
		[],
	);

	const revealInFinder = useCallback(async (path: string) => {
		await revealItemInDir(path);
	}, []);

	const fileExists = useCallback(async (path: string) => {
		return exists(path);
	}, []);

	const cut = useCallback((path: string, type: "file" | "folder") => {
		setClipboard({ operation: "cut", sourcePath: path, type });
	}, []);

	const copy = useCallback((path: string, type: "file" | "folder") => {
		setClipboard({ operation: "copy", sourcePath: path, type });
	}, []);

	const paste = useCallback(
		async (destDir: string) => {
			if (!clipboard) return;

			const fileName = clipboard.sourcePath.split("/").pop() ?? "";
			const destPath = `${destDir}/${fileName}`;

			if (clipboard.operation === "cut") {
				await rename(clipboard.sourcePath, destPath);
				setClipboard(null);
			} else {
				if (clipboard.type === "folder") {
					await copyDirRecursive(clipboard.sourcePath, destPath);
				} else {
					await copyFile(clipboard.sourcePath, destPath);
				}
			}
		},
		[clipboard],
	);

	return {
		clipboard,
		createFile,
		createFolder,
		deleteItem,
		renameItem,
		copyPath,
		copyRelativePath,
		revealInFinder,
		fileExists,
		cut,
		copy,
		paste,
	};
}
