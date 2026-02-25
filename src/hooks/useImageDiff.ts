import { invoke } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import { useEffect, useState } from "react";
import type { DiffBase } from "@/components/panels/MonacoDiffViewer";
import { buildDataUrl, getMimeType } from "@/lib/imageUtils";

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

		const fetchModified = async () => {
			try {
				const bytes = await readFile(filePath);
				if (!cancelled) {
					const base64 = uint8ArrayToBase64(bytes);
					setModifiedUrl(buildDataUrl(base64, mime));
				}
			} catch {
				if (!cancelled) setModifiedUrl(null);
			}
		};

		const fetchOriginal = async () => {
			try {
				let base64: string;
				if (diffBase === "staged") {
					base64 = await invoke<string>("get_binary_staged_content", {
						filePath,
					});
				} else {
					base64 = await invoke<string>("get_binary_file_at_branch_base", {
						filePath,
					});
				}
				if (!cancelled) {
					setOriginalUrl(buildDataUrl(base64, mime));
				}
			} catch {
				if (!cancelled) setOriginalUrl(null);
			}
		};

		Promise.all([fetchModified(), fetchOriginal()]).finally(() => {
			if (!cancelled) setLoading(false);
		});

		return () => {
			cancelled = true;
		};
	}, [filePath, diffBase, gitRefreshKey]);

	return { originalUrl, modifiedUrl, loading };
}
