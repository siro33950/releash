import { useCallback, useState } from "react";
import { normalizePath } from "@/lib/normalizePath";

const STORAGE_KEY = "releash-repos";

function loadRepoPaths(): string[] {
	try {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored) {
			const parsed = JSON.parse(stored);
			if (Array.isArray(parsed)) {
				return parsed
					.filter((v): v is string => typeof v === "string")
					.map(normalizePath);
			}
		}
	} catch {
		// ignore
	}
	return [];
}

function saveRepoPaths(paths: string[]): void {
	localStorage.setItem(STORAGE_KEY, JSON.stringify(paths));
}

export interface UseRepoListReturn {
	repoPaths: string[];
	addRepo: (path: string) => void;
	removeRepo: (path: string) => void;
	initFromCwd: (cwdRepoPath: string) => void;
}

export function useRepoList(): UseRepoListReturn {
	const [repoPaths, setRepoPaths] = useState<string[]>(loadRepoPaths);

	const addRepo = useCallback((path: string) => {
		const normalized = normalizePath(path);
		setRepoPaths((prev) => {
			if (prev.includes(normalized)) return prev;
			const next = [...prev, normalized];
			saveRepoPaths(next);
			return next;
		});
	}, []);

	const removeRepo = useCallback((path: string) => {
		const normalized = normalizePath(path);
		setRepoPaths((prev) => {
			const next = prev.filter((p) => p !== normalized);
			saveRepoPaths(next);
			return next;
		});
	}, []);

	const initFromCwd = useCallback((cwdRepoPath: string) => {
		const normalized = normalizePath(cwdRepoPath);
		setRepoPaths((prev) => {
			if (prev.includes(normalized)) return prev;
			const next = [normalized, ...prev];
			saveRepoPaths(next);
			return next;
		});
	}, []);

	return { repoPaths, addRepo, removeRepo, initFromCwd };
}
