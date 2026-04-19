import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { DiffTreeNode } from "@/hooks/useDiffFileTree";

export interface FileNavigationResult {
	current_index: number;
	total: number;
	prev_file: string | null;
	next_file: string | null;
}

const EMPTY_NAVIGATION: FileNavigationResult = {
	current_index: 0,
	total: 0,
	prev_file: null,
	next_file: null,
};

export function useFileNavigation(
	tree: DiffTreeNode[],
	currentFile: string | null,
) {
	const [navigation, setNavigation] =
		useState<FileNavigationResult>(EMPTY_NAVIGATION);

	useEffect(() => {
		if (!currentFile || tree.length === 0) {
			setNavigation(EMPTY_NAVIGATION);
			return;
		}

		invoke<FileNavigationResult>("get_file_navigation", {
			tree,
			currentFile,
		})
			.then(setNavigation)
			.catch(() => setNavigation(EMPTY_NAVIGATION));
	}, [tree, currentFile]);

	const goToPrevFile = useCallback(() => {
		return navigation.prev_file;
	}, [navigation.prev_file]);

	const goToNextFile = useCallback(() => {
		return navigation.next_file;
	}, [navigation.next_file]);

	return {
		fileNavigation: navigation,
		goToPrevFile,
		goToNextFile,
	};
}
