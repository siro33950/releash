import { type MutableRefObject, useMemo } from "react";
import { type MenuHandlers, useMenuEvents } from "@/hooks/useMenuEvents";
import type { AppSettings } from "@/types/settings";
import type { UIAction, WorktreeGitActions } from "./useWorktreeGitActions";

interface UseWorktreeMenuHandlersParams {
	dispatchUI: React.Dispatch<UIAction>;
	settingsRef: MutableRefObject<AppSettings>;
	onSettingsSaveRef: MutableRefObject<(settings: AppSettings) => void>;
	gitActions: WorktreeGitActions;
	isActive: boolean;
}

export function useWorktreeMenuHandlers({
	dispatchUI,
	settingsRef,
	onSettingsSaveRef,
	gitActions,
	isActive,
}: UseWorktreeMenuHandlersParams) {
	const menuHandlers: MenuHandlers = useMemo(
		() => ({
			settings: () => dispatchUI({ type: "SET_SETTINGS_OPEN", open: true }),
			"increase-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({
					...s,
					fontSize: Math.min(24, s.fontSize + 1),
				});
			},
			"decrease-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({
					...s,
					fontSize: Math.max(8, s.fontSize - 1),
				});
			},
			"reset-font-size": () => {
				const s = settingsRef.current;
				onSettingsSaveRef.current({ ...s, fontSize: 14 });
			},
			"git-stage-all": gitActions.handleGitStageAll,
			"git-unstage-all": gitActions.handleGitUnstageAll,
			"git-create-branch": gitActions.handleGitCreateBranch,
			"new-terminal": () => {},
		}),
		[dispatchUI, settingsRef, onSettingsSaveRef, gitActions],
	);

	useMenuEvents(menuHandlers, isActive);
}
