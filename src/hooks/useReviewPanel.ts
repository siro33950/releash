import { useCallback, useState } from "react";
import type { DiffBase, DiffMode, DiffSection } from "@/types/settings";

export interface UseReviewPanelOptions {
	initialDiffBase?: DiffBase;
	initialDiffMode?: DiffMode;
}

export interface UseReviewPanelResult {
	diffBase: DiffBase;
	diffMode: DiffMode;
	selectedFile: string | null;
	selectedSection: DiffSection;
	setDiffBase: (base: DiffBase) => void;
	setDiffMode: (mode: DiffMode) => void;
	selectFile: (path: string | null, section?: DiffSection) => void;
}

export function useReviewPanel(
	options?: UseReviewPanelOptions,
): UseReviewPanelResult {
	const [diffBase, setDiffBase] = useState<DiffBase>(
		options?.initialDiffBase ?? "head",
	);
	const [diffMode, setDiffMode] = useState<DiffMode>(
		options?.initialDiffMode ?? "inline",
	);
	const [selectedFile, setSelectedFile] = useState<string | null>(null);
	const [selectedSection, setSelectedSection] =
		useState<DiffSection>("changes");

	const selectFile = useCallback(
		(path: string | null, section?: DiffSection) => {
			setSelectedFile(path);
			if (section) {
				setSelectedSection(section);
			}
		},
		[],
	);

	return {
		diffBase,
		diffMode,
		selectedFile,
		selectedSection,
		setDiffBase,
		setDiffMode,
		selectFile,
	};
}
