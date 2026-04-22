import { useEffect, useState } from "react";
import type { Subscribe } from "./useMessageBus";

interface UseRemoteContentOptions {
	subscribe: Subscribe;
}

export function useRemoteContent({ subscribe }: UseRemoteContentOptions) {
	const [branchName, setBranchName] = useState<string | null>(null);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "branch_info_response") {
				setBranchName(msg.payload.branch);
			}
		});
	}, [subscribe]);

	return {
		branchName,
		setBranchName,
	};
}
