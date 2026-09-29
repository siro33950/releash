import { useCallback, useEffect, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type { DiffTreeNode } from "@/types/review";

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
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		if (!currentFile || tree.length === 0) {
			setNavigation(EMPTY_NAVIGATION);
			setError(null);
			return;
		}
		setError(null);

		invoke("get_file_navigation", {
			tree,
			currentFile,
		})
			.then((result) => {
				if (active) setNavigation(result);
			})
			.catch((cause) => {
				if (!active) return;
				setNavigation(EMPTY_NAVIGATION);
				setError(getErrorMessage(cause));
			});
		return () => {
			active = false;
		};
	}, [tree, currentFile]);

	const goToPrevFile = useCallback(() => {
		return navigation.prev_file;
	}, [navigation.prev_file]);

	const goToNextFile = useCallback(() => {
		return navigation.next_file;
	}, [navigation.next_file]);

	return {
		fileNavigation: navigation,
		error,
		goToPrevFile,
		goToNextFile,
	};
}
