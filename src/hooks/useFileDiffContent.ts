import { invoke } from "@tauri-apps/api/core";
import { readTextFile } from "@tauri-apps/plugin-fs";
import { useEffect, useRef, useState } from "react";
import type { DiffBase, DiffSection } from "@/types/settings";

export function useFileDiffContent(
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
	gitRefreshKey: number,
) {
	const [originalContent, setOriginalContent] = useState("");
	const [modifiedContent, setModifiedContent] = useState("");
	const [loading, setLoading] = useState(false);
	const requestIdRef = useRef(0);

	// biome-ignore lint/correctness/useExhaustiveDependencies: gitRefreshKey is an intentional trigger to re-fetch on git state changes
	useEffect(() => {
		const requestId = ++requestIdRef.current;

		if (!filePath) {
			setOriginalContent("");
			setModifiedContent("");
			setLoading(false);
			return;
		}

		setLoading(true);

		const fetchStaged = async () => {
			try {
				return await invoke<string>("get_staged_content", { filePath });
			} catch {
				return "";
			}
		};

		const fetchHead = async () => {
			try {
				return await invoke<string>("get_file_at_ref", {
					filePath,
					gitRef: "HEAD",
				});
			} catch {
				return "";
			}
		};

		const fetchBranchBase = async () => {
			try {
				return await invoke<string>("get_file_at_branch_base", {
					filePath,
				});
			} catch {
				return "";
			}
		};

		const fetchWorkingTree = async () => {
			try {
				return await readTextFile(filePath);
			} catch {
				return "";
			}
		};

		let fetchPair: Promise<[string, string]>;

		if (diffBase === "branch-base") {
			fetchPair = Promise.all([fetchBranchBase(), fetchWorkingTree()]);
		} else if (section === "staged") {
			// Staged Changes: HEAD → Staged
			fetchPair = Promise.all([fetchHead(), fetchStaged()]);
		} else {
			// Changes: Staged → Working Tree
			// When both staged and working tree are empty (deleted file),
			// fall back to HEAD for original content
			fetchPair = Promise.all([fetchStaged(), fetchWorkingTree()]).then(
				async ([staged, workingTree]) => {
					if (staged === "" && workingTree === "") {
						const head = await fetchHead();
						return [head, workingTree] as [string, string];
					}
					return [staged, workingTree] as [string, string];
				},
			);
		}

		fetchPair
			.then(([original, modified]) => {
				if (requestId !== requestIdRef.current) return;
				setOriginalContent(original);
				setModifiedContent(modified);
				setLoading(false);
			})
			.catch(() => {
				if (requestId !== requestIdRef.current) return;
				setOriginalContent("");
				setModifiedContent("");
				setLoading(false);
			});

		return () => {
			requestIdRef.current++;
		};
	}, [filePath, diffBase, section, gitRefreshKey]);

	return { originalContent, modifiedContent, loading };
}
