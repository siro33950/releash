import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ReviewSnapshot } from "@/types/review";
import type { DiffBase } from "@/types/settings";

const EMPTY_SNAPSHOT: ReviewSnapshot = {
	version: 0,
	stale: false,
	loading: false,
	limited: false,
	base: "head",
	files: [],
	stagedFiles: [],
	changedFiles: [],
	diffStats: [],
	tree: [],
	stagedTree: [],
	changesTree: [],
	stagedFileCount: 0,
	changesFileCount: 0,
};

export function useReviewSnapshot(
	rootPath: string | null,
	diffBase: DiffBase,
	externalRefreshKey?: number,
) {
	const [snapshot, setSnapshot] = useState<ReviewSnapshot>(EMPTY_SNAPSHOT);
	const [loading, setLoading] = useState(false);
	const acceptedVersionRef = useRef<number | null>(null);
	const activeInputKeyRef = useRef("");
	const requestIdRef = useRef(0);
	const snapshotInputKeyRef = useRef<string | null>(null);
	const inputKey = `${rootPath ?? ""}\0${diffBase}`;

	activeInputKeyRef.current = inputKey;

	useEffect(() => {
		acceptedVersionRef.current = null;
		requestIdRef.current += 1;
		snapshotInputKeyRef.current = inputKey;
		setSnapshot({ ...EMPTY_SNAPSHOT, base: diffBase });
		setLoading(false);
	}, [inputKey, diffBase]);

	const fetchSnapshot = useCallback(async () => {
		const requestId = ++requestIdRef.current;
		const requestInputKey = inputKey;
		if (!rootPath) {
			acceptedVersionRef.current = null;
			snapshotInputKeyRef.current = requestInputKey;
			setSnapshot({ ...EMPTY_SNAPSHOT, base: diffBase });
			setLoading(false);
			return;
		}

		setLoading(true);
		try {
			const result = await invoke<ReviewSnapshot>("get_review_snapshot", {
				input: { worktreePath: rootPath, base: diffBase },
			});
			if (
				requestId !== requestIdRef.current ||
				requestInputKey !== activeInputKeyRef.current
			) {
				return;
			}
			if (
				acceptedVersionRef.current != null &&
				result.version < acceptedVersionRef.current
			) {
				return;
			}
			acceptedVersionRef.current = result.version;
			snapshotInputKeyRef.current = requestInputKey;
			setSnapshot(result);
		} catch {
			if (
				requestId !== requestIdRef.current ||
				requestInputKey !== activeInputKeyRef.current
			) {
				return;
			}
			acceptedVersionRef.current = null;
			snapshotInputKeyRef.current = requestInputKey;
			setSnapshot({ ...EMPTY_SNAPSHOT, base: diffBase });
		} finally {
			if (
				requestId === requestIdRef.current &&
				requestInputKey === activeInputKeyRef.current
			) {
				setLoading(false);
			}
		}
	}, [rootPath, diffBase, inputKey]);

	useEffect(() => {
		fetchSnapshot();
	}, [fetchSnapshot]);

	useEffect(() => {
		if (externalRefreshKey != null && externalRefreshKey > 0) {
			fetchSnapshot();
		}
	}, [externalRefreshKey, fetchSnapshot]);

	const visibleSnapshot =
		snapshotInputKeyRef.current === inputKey
			? snapshot
			: { ...EMPTY_SNAPSHOT, base: diffBase };

	return {
		snapshot: visibleSnapshot,
		files: visibleSnapshot.files,
		stagedFiles: visibleSnapshot.stagedFiles,
		changedFiles: visibleSnapshot.changedFiles,
		stagedTree: visibleSnapshot.stagedTree,
		changesTree: visibleSnapshot.changesTree,
		branchBaseTree: visibleSnapshot.tree,
		stagedFileCount: visibleSnapshot.stagedFileCount,
		changesFileCount: visibleSnapshot.changesFileCount,
		branchBaseFileCount: visibleSnapshot.files.length,
		version: visibleSnapshot.version,
		limited: visibleSnapshot.limited,
		loading,
		refresh: fetchSnapshot,
	};
}
