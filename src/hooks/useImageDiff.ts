import { invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { useEffect, useState } from "react";
import { buildDataUrl, getMimeType } from "@/lib/imageUtils";
import type { DiffBase, DiffSection } from "@/types/settings";

export interface ImageDiffResult {
	originalUrl: string | null;
	modifiedUrl: string | null;
	loading: boolean;
}

function uint8ArrayToBase64(bytes: Uint8Array): string {
	let binary = "";
	for (let i = 0; i < bytes.length; i++) {
		binary += String.fromCharCode(bytes[i]);
	}
	return btoa(binary);
}

export function useImageDiff(
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
	gitRefreshKey?: number,
): ImageDiffResult {
	const [originalUrl, setOriginalUrl] = useState<string | null>(null);
	const [modifiedUrl, setModifiedUrl] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: gitRefreshKey is an intentional trigger to re-fetch images on git state changes
	useEffect(() => {
		if (!filePath) {
			setOriginalUrl(null);
			setModifiedUrl(null);
			return;
		}

		let cancelled = false;
		setLoading(true);

		const mime = getMimeType(filePath);

		const fetchWorkingTree = async (): Promise<string | null> => {
			try {
				const bytes = await readFile(filePath);
				const base64 = uint8ArrayToBase64(bytes);
				return buildDataUrl(base64, mime);
			} catch {
				return null;
			}
		};

		const fetchStaged = async (): Promise<string | null> => {
			try {
				const base64 = await invoke<string>("get_binary_staged_content", {
					filePath,
				});
				return buildDataUrl(base64, mime);
			} catch {
				return null;
			}
		};

		const fetchHead = async (): Promise<string | null> => {
			try {
				const base64 = await invoke<string>("get_binary_file_at_ref", {
					filePath,
					gitRef: "HEAD",
				});
				return buildDataUrl(base64, mime);
			} catch {
				return null;
			}
		};

		const fetchBranchBase = async (): Promise<string | null> => {
			try {
				const base64 = await invoke<string>("get_binary_file_at_branch_base", {
					filePath,
				});
				return buildDataUrl(base64, mime);
			} catch {
				return null;
			}
		};

		const fetchAll = async () => {
			let original: string | null;
			let modified: string | null;

			if (diffBase === "branch-base") {
				[original, modified] = await Promise.all([
					fetchBranchBase(),
					fetchWorkingTree(),
				]);
			} else if (section === "staged") {
				// Staged Changes: HEAD → Staged
				[original, modified] = await Promise.all([fetchHead(), fetchStaged()]);
			} else {
				// Changes: Staged → Working Tree
				[original, modified] = await Promise.all([
					fetchStaged(),
					fetchWorkingTree(),
				]);
			}

			if (!cancelled) {
				setOriginalUrl(original);
				setModifiedUrl(modified);
				setLoading(false);
			}
		};

		fetchAll();

		return () => {
			cancelled = true;
		};
	}, [filePath, diffBase, section, gitRefreshKey]);

	return { originalUrl, modifiedUrl, loading };
}
