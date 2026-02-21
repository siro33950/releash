import { useCallback } from "react";
import {
	type ChangeGroup,
	computeChangeGroups,
	computeHunks,
	type Hunk,
} from "@/lib/computeHunks";
import { generateGroupPatch, generatePatch } from "@/lib/generatePatch";
import type { DiffBase } from "@/types/settings";

export interface UseDiffOperationsParams {
	filePath: string;
	rootPath: string | null;
	originalContent: string;
	modifiedContent: string;
	stagedContent: string;
	diffBase: DiffBase;
	onStageHunk?: (rootPath: string, patch: string) => Promise<void>;
	onGitChanged?: () => void;
}

export interface UseDiffOperationsResult {
	handleStageGroup: (groupIndex: number) => Promise<void>;
	handleUnstageGroup: (groupIndex: number) => Promise<void>;
	handleStageAll: () => Promise<void>;
	handleUnstageAll: () => Promise<void>;
}

function getRelativePath(rootPath: string | null, filePath: string) {
	if (!rootPath) return null;
	return filePath.startsWith(`${rootPath}/`)
		? filePath.slice(rootPath.length + 1)
		: filePath;
}

function findMatchingGroup(
	targetLines: string[],
	hunks: Hunk[],
	groups: ChangeGroup[],
	reverse = false,
) {
	let target: string;
	if (reverse) {
		const newMinus = targetLines
			.filter((l) => l.startsWith("+"))
			.map((l) => `-${l.slice(1)}`);
		const newPlus = targetLines
			.filter((l) => l.startsWith("-"))
			.map((l) => `+${l.slice(1)}`);
		target = [...newMinus, ...newPlus].join("\n");
	} else {
		target = targetLines.join("\n");
	}
	for (const g of groups) {
		const h = hunks.find((h) => h.index === g.hunkIndex);
		if (!h) continue;
		const lines = h.lines
			.slice(g.lineOffsetStart, g.lineOffsetEnd + 1)
			.join("\n");
		if (lines === target) return { group: g, hunk: h };
	}
	return null;
}

export function useDiffOperations({
	filePath,
	rootPath,
	originalContent,
	modifiedContent,
	stagedContent,
	diffBase,
	onStageHunk,
	onGitChanged,
}: UseDiffOperationsParams): UseDiffOperationsResult {
	const handleStageGroup = useCallback(
		async (groupIndex: number) => {
			const relativePath = getRelativePath(rootPath, filePath);
			if (!relativePath || !rootPath) return;
			const allHunks = computeHunks(
				originalContent,
				modifiedContent,
				relativePath,
			);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			let patchHunk = hunk;
			let patchGroup = group;

			if (diffBase === "HEAD") {
				const targetLines = hunk.lines.slice(
					group.lineOffsetStart,
					group.lineOffsetEnd + 1,
				);
				const s2wHunks = computeHunks(
					stagedContent,
					modifiedContent,
					relativePath,
				);
				const s2wGroups = computeChangeGroups(s2wHunks);
				const match = findMatchingGroup(targetLines, s2wHunks, s2wGroups);
				if (!match) return;
				patchHunk = match.hunk;
				patchGroup = match.group;
			}

			const patch = generateGroupPatch(relativePath, patchHunk, patchGroup);
			if (patch) {
				try {
					await onStageHunk?.(rootPath, patch);
					onGitChanged?.();
				} catch (e) {
					console.error("Stage group failed:", e);
				}
			}
		},
		[
			rootPath,
			filePath,
			originalContent,
			modifiedContent,
			stagedContent,
			diffBase,
			onStageHunk,
			onGitChanged,
		],
	);

	const handleUnstageGroup = useCallback(
		async (groupIndex: number) => {
			const relativePath = getRelativePath(rootPath, filePath);
			if (!relativePath || !rootPath) return;
			const allHunks = computeHunks(
				originalContent,
				modifiedContent,
				relativePath,
			);
			const allGroups = computeChangeGroups(allHunks);
			const group = allGroups.find((g) => g.groupIndex === groupIndex);
			if (!group) return;
			const hunk = allHunks.find((h) => h.index === group.hunkIndex);
			if (!hunk) return;

			const targetLines = hunk.lines.slice(
				group.lineOffsetStart,
				group.lineOffsetEnd + 1,
			);
			const s2hHunks = computeHunks(
				stagedContent,
				originalContent,
				relativePath,
			);
			const s2hGroups = computeChangeGroups(s2hHunks);
			const match = findMatchingGroup(targetLines, s2hHunks, s2hGroups, true);
			if (!match) return;

			const patch = generateGroupPatch(relativePath, match.hunk, match.group);
			if (patch) {
				try {
					await onStageHunk?.(rootPath, patch);
					onGitChanged?.();
				} catch (e) {
					console.error("Unstage group failed:", e);
				}
			}
		},
		[
			rootPath,
			filePath,
			originalContent,
			modifiedContent,
			stagedContent,
			onStageHunk,
			onGitChanged,
		],
	);

	const handleStageAll = useCallback(async () => {
		const relativePath = getRelativePath(rootPath, filePath);
		if (!relativePath || !rootPath) return;
		const base = diffBase === "HEAD" ? stagedContent : originalContent;
		const allHunks = computeHunks(base, modifiedContent, relativePath);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(relativePath, allHunks, allIndices);
		if (patch) {
			try {
				await onStageHunk?.(rootPath, patch);
				onGitChanged?.();
			} catch (e) {
				console.error("Stage all failed:", e);
			}
		}
	}, [
		rootPath,
		filePath,
		originalContent,
		modifiedContent,
		stagedContent,
		diffBase,
		onStageHunk,
		onGitChanged,
	]);

	const handleUnstageAll = useCallback(async () => {
		const relativePath = getRelativePath(rootPath, filePath);
		if (!relativePath || !rootPath || !stagedContent) return;
		const allHunks = computeHunks(stagedContent, originalContent, relativePath);
		const allIndices = allHunks.map((h) => h.index);
		const patch = generatePatch(relativePath, allHunks, allIndices);
		if (patch) {
			try {
				await onStageHunk?.(rootPath, patch);
				onGitChanged?.();
			} catch (e) {
				console.error("Unstage all failed:", e);
			}
		}
	}, [
		rootPath,
		filePath,
		originalContent,
		stagedContent,
		onStageHunk,
		onGitChanged,
	]);

	return {
		handleStageGroup,
		handleUnstageGroup,
		handleStageAll,
		handleUnstageAll,
	};
}
