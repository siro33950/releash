import { useEffect, useRef, useState } from "react";
import type { PanelImperativeHandle } from "react-resizable-panels";
import { useWorkspaceStateCache } from "@/hooks/useWorkspaceStateCache";
import {
	buildWorkspaceState,
	type InternalWorktreeState,
	type WorkspaceState,
} from "@/types/workspace-state";

interface UseWorkspacePersistenceParams {
	selectedRootPath: string | null;
	centerTab: string;
	leftNavVisible: boolean;
	rightVisible: boolean;
	setCenterTab: (tab: string) => void;
	leftNavRef: React.RefObject<PanelImperativeHandle | null>;
	rightPanelRef: React.RefObject<PanelImperativeHandle | null>;
}

interface UseWorkspacePersistenceReturn {
	internalStateMapRef: React.MutableRefObject<
		Map<string, InternalWorktreeState>
	>;
	getInitialState: (rootPath: string) => WorkspaceState | undefined;
	stateReady: boolean;
}

export function useWorkspacePersistence({
	selectedRootPath,
	centerTab,
	leftNavVisible,
	rightVisible,
	setCenterTab,
	leftNavRef,
	rightPanelRef,
}: UseWorkspacePersistenceParams): UseWorkspacePersistenceReturn {
	const workspaceCache = useWorkspaceStateCache();
	const internalStateMapRef = useRef<Map<string, InternalWorktreeState>>(
		new Map(),
	);

	const [stateReady, setStateReady] = useState(() => {
		if (!selectedRootPath) return true;
		return !!workspaceCache.getState(selectedRootPath);
	});

	const centerTabRef = useRef(centerTab);
	centerTabRef.current = centerTab;
	const leftNavVisibleRef = useRef(leftNavVisible);
	leftNavVisibleRef.current = leftNavVisible;
	const rightVisibleRef = useRef(rightVisible);
	rightVisibleRef.current = rightVisible;
	const workspaceCacheRef = useRef(workspaceCache);
	workspaceCacheRef.current = workspaceCache;

	// Render中にselectedRootPathの変更を検知してcenterTabを同期更新
	const [prevPath, setPrevPath] = useState(selectedRootPath);
	if (selectedRootPath !== prevPath) {
		setPrevPath(selectedRootPath);

		// Save previous worktree's state (Mapから読むので安全)
		if (prevPath) {
			const internal = internalStateMapRef.current.get(prevPath);
			if (internal) {
				const state = buildWorkspaceState(
					internal,
					centerTabRef.current,
					leftNavVisibleRef.current,
					rightVisibleRef.current,
				);
				workspaceCache.updateState(prevPath, state);
				workspaceCache.flushState(prevPath);
				internalStateMapRef.current.delete(prevPath);
			}
		}

		// centerTabの同期復元
		const cached = selectedRootPath
			? workspaceCache.getState(selectedRootPath)
			: undefined;
		setCenterTab(cached?.layout.centerTab ?? "agent");

		// 切替先のキャッシュ有無でstateReadyを更新
		const hasCache = selectedRootPath
			? !!workspaceCache.getState(selectedRootPath)
			: true;
		setStateReady(hasCache);
	}

	// Restore panel expand/collapse (DOM操作のためuseEffect + rAF)
	useEffect(() => {
		if (!selectedRootPath) return;
		const cache = workspaceCacheRef.current;
		const cached = cache.getState(selectedRootPath);
		if (cached) {
			requestAnimationFrame(() => {
				if (cached.layout.leftNavCollapsed) {
					leftNavRef.current?.collapse();
				} else {
					leftNavRef.current?.expand();
				}
				if (cached.layout.rightCollapsed) {
					rightPanelRef.current?.collapse();
				} else {
					rightPanelRef.current?.expand();
				}
			});
		} else {
			// Default reset: no cached state → expand panels
			requestAnimationFrame(() => {
				leftNavRef.current?.expand();
				rightPanelRef.current?.expand();
			});
		}
	}, [selectedRootPath, leftNavRef, rightPanelRef]);

	// Pre-load workspace state on first mount
	useEffect(() => {
		if (!selectedRootPath) return;
		const cache = workspaceCacheRef.current;
		if (cache.getState(selectedRootPath)) return;
		cache.loadState(selectedRootPath).then(() => {
			setStateReady(true);
		});
	}, [selectedRootPath]);

	return {
		internalStateMapRef,
		getInitialState: workspaceCache.getState,
		stateReady,
	};
}
