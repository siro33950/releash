import { createContext, useCallback, useMemo, useState } from "react";
import type { WorkspaceListSnapshotDto } from "@/generated/client_types";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useStateSubscriptionResult } from "./useStateSubscription";

export interface WorkspaceListModel {
	snapshot: WorkspaceListSnapshotDto | null;
	requestError: { message: string } | null;
	refresh: () => Promise<void>;
}
export const WorkspaceListContext = createContext<WorkspaceListModel | null>(
	null,
);

export function useWorkspaceList(): WorkspaceListModel {
	const subscription = useStateSubscriptionResult("workspaces");
	const snapshot = subscription.value ?? null;
	const [requestError, setRequestError] =
		useState<WorkspaceListModel["requestError"]>(null);
	const refresh = useCallback(async () => {
		try {
			await invoke("refresh_workspaces", {});
			setRequestError(null);
		} catch (error) {
			setRequestError({ message: getErrorMessage(error) });
		}
	}, []);
	return useMemo(
		() => ({
			snapshot,
			requestError:
				requestError ??
				(subscription.error ? { message: subscription.error } : null),
			refresh,
		}),
		[snapshot, requestError, refresh, subscription.error],
	);
}
