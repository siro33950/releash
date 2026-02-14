import { useCallback, useEffect, useRef } from "react";

interface UseBrowserBackGuardOptions {
	selectedWorktree: string | null;
	onBack: () => void;
}

/**
 * ブラウザの戻る操作をインターセプトし、Remote UI内のナビゲーションとして処理する。
 * モバイルブラウザでスワイプバック等を行った際にページ遷移（WebSocket接続切断）を防ぐ。
 *
 * 仕組み:
 * - history.pushState でガードエントリを積み、ブラウザバックでページが離脱しないようにする
 * - Worktree選択時に追加のエントリを積み、戻る操作でダッシュボードに戻れるようにする
 * - popstate イベントでブラウザバックを検知し、アプリ内ナビゲーションとして処理する
 */
export function useBrowserBackGuard({
	selectedWorktree,
	onBack,
}: UseBrowserBackGuardOptions) {
	const selectedWorktreeRef = useRef(selectedWorktree);
	const onBackRef = useRef(onBack);

	useEffect(() => {
		selectedWorktreeRef.current = selectedWorktree;
	}, [selectedWorktree]);

	useEffect(() => {
		onBackRef.current = onBack;
	}, [onBack]);

	useEffect(() => {
		const guardState = { _remoteGuard: true };
		history.scrollRestoration = "manual";
		history.replaceState(guardState, "");
		history.pushState(guardState, "");

		const handlePopState = () => {
			if (selectedWorktreeRef.current) {
				onBackRef.current();
			}
			history.pushState(guardState, "");
		};

		window.addEventListener("popstate", handlePopState);
		return () => window.removeEventListener("popstate", handlePopState);
	}, []);

	useEffect(() => {
		if (selectedWorktree) {
			history.pushState({ _remoteWorktree: true }, "");
		}
	}, [selectedWorktree]);

	const navigateBack = useCallback(() => {
		history.back();
	}, []);

	return { navigateBack };
}
