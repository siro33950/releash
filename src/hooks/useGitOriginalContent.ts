import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import type { DiffBase } from "@/components/panels/MonacoDiffViewer";
import type { FileChangeEvent } from "@/hooks/useFileWatcher";
import { normalizePath } from "@/lib/normalizePath";

export function useGitOriginalContent(
	filePath: string | null,
	diffBase: DiffBase,
	fallbackContent: string,
): string {
	const [originalContent, setOriginalContent] = useState(fallbackContent);
	const [refreshKey, setRefreshKey] = useState(0);
	const [prevFilePath, setPrevFilePath] = useState(filePath);
	const gitIndexPathRef = useRef<string | null>(null);

	// ファイル切り替え時に即座にリセット（Reactがレンダーを中断し新しいstateで再レンダー）
	if (filePath !== prevFilePath) {
		setPrevFilePath(filePath);
		setOriginalContent(fallbackContent);
	}
	const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(() => {
		if (!filePath) {
			gitIndexPathRef.current = null;
			return;
		}

		let cancelled = false;

		invoke<string>("get_repo_git_dir", { filePath })
			.then((gitDir) => {
				if (!cancelled) {
					const normalized = normalizePath(gitDir);
					const indexPath = normalized.endsWith("/")
						? `${normalized}index`
						: `${normalized}/index`;
					gitIndexPathRef.current = indexPath;
				}
			})
			.catch(() => {
				if (!cancelled) {
					gitIndexPathRef.current = null;
				}
			});

		return () => {
			cancelled = true;
		};
	}, [filePath]);

	useEffect(() => {
		const unlisten$ = listen<FileChangeEvent>("file-change", (event) => {
			const changedPath = normalizePath(event.payload.path);
			if (gitIndexPathRef.current && changedPath === gitIndexPathRef.current) {
				if (debounceTimerRef.current) {
					clearTimeout(debounceTimerRef.current);
				}
				debounceTimerRef.current = setTimeout(() => {
					setRefreshKey((k) => k + 1);
				}, 300);
			}
		});

		return () => {
			unlisten$.then((fn) => fn());
			if (debounceTimerRef.current) {
				clearTimeout(debounceTimerRef.current);
			}
		};
	}, []);

	useEffect(() => {
		void refreshKey;

		if (!filePath) {
			setOriginalContent(fallbackContent);
			return;
		}

		let cancelled = false;

		const fetchContent = async () => {
			try {
				let content: string;
				if (diffBase === "staged") {
					content = await invoke<string>("get_staged_content", {
						filePath,
					});
				} else {
					content = await invoke<string>("get_file_at_ref", {
						filePath,
						gitRef: diffBase,
					});
				}
				if (!cancelled) {
					setOriginalContent(content);
				}
			} catch {
				if (!cancelled) {
					setOriginalContent("");
				}
			}
		};

		fetchContent();

		return () => {
			cancelled = true;
		};
	}, [filePath, diffBase, fallbackContent, refreshKey]);

	return originalContent;
}
