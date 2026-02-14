import { useCallback, useEffect, useRef } from "react";

interface UseBrowserBackGuardOptions {
	selectedWorktree: string | null;
	onBack: () => void;
}

interface NavigateEvent extends Event {
	navigationType: string;
	intercept(options?: {
		handler?: () => Promise<void>;
		focusReset?: "after-transition" | "manual";
		scroll?: "after-transition" | "manual";
	}): void;
	canIntercept: boolean;
}

interface AppNavigation {
	addEventListener(
		type: "navigate",
		listener: (event: NavigateEvent) => void,
	): void;
	removeEventListener(
		type: "navigate",
		listener: (event: NavigateEvent) => void,
	): void;
}

function getNavigation(): AppNavigation | null {
	if ("navigation" in window) {
		return (window as unknown as { navigation: AppNavigation }).navigation;
	}
	return null;
}

/**
 * ブラウザの戻る操作をインターセプトし、Remote UI内のナビゲーションとして処理する。
 * モバイルブラウザでスワイプバック等を行った際にページ遷移（WebSocket接続切断）を防ぐ。
 *
 * Navigation API対応ブラウザ (Chrome 102+):
 * - navigate イベントで traverse ナビゲーションをインターセプトし、
 *   ブラウザのデフォルトアニメーションを抑制してアプリ内ナビゲーションとして処理する。
 *
 * 非対応ブラウザ (Safari等):
 * - history.pushState でガードエントリを積み、popstate イベントで処理する。
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

		const nav = getNavigation();

		if (nav) {
			const handleNavigate = (event: NavigateEvent) => {
				if (event.navigationType !== "traverse" || !event.canIntercept) {
					return;
				}
				event.intercept({
					handler: async () => {
						if (selectedWorktreeRef.current) {
							onBackRef.current();
						}
					},
					focusReset: "manual",
					scroll: "manual",
				});
				// 次のバック操作に備えてガードを再追加
				queueMicrotask(() => {
					history.pushState(guardState, "");
				});
			};

			nav.addEventListener("navigate", handleNavigate);
			return () => nav.removeEventListener("navigate", handleNavigate);
		}

		// Navigation API非対応ブラウザ向けフォールバック
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
