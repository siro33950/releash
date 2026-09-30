import { useCallback, useEffect, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { showClientError } from "@/lib/clientErrorNotice";
import type { NotionLabelOption } from "@/types/notion";

export function useNotionLabelOptions(repoPath: string) {
	const [labelOptions, setLabelOptions] = useState<NotionLabelOption[]>([]);
	const [loading, setLoading] = useState(true);

	const fetchOptions = useCallback(async () => {
		setLoading(true);
		try {
			const result = await invoke("fetch_notion_label_options", { repoPath });
			setLabelOptions(result);
		} catch (error) {
			showClientError(error);
			setLabelOptions([]);
		} finally {
			setLoading(false);
		}
	}, [repoPath]);

	useEffect(() => {
		fetchOptions();
	}, [fetchOptions]);

	return { labelOptions, loading };
}
